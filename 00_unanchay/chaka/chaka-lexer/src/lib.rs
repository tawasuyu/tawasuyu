//! `chaka-lexer` — tokenizador de COBOL.
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

/// Dialecto fuente del lexer. La v1 sólo implementa `Cobol`; las
/// variantes futuras se conectarán aquí sin romper el API (`lex` queda
/// como un atajo a `Dialect::Cobol`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Dialect {
    /// COBOL'85 (subconjunto), el dialecto principal del transpilador.
    Cobol,
}

impl Default for Dialect {
    fn default() -> Self {
        Self::Cobol
    }
}

impl Dialect {
    /// Adivina el dialecto por la extensión del fichero (case-insensitive).
    /// `None` si no la reconoce.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "cob" | "cbl" | "cpy" | "cobol" => Some(Self::Cobol),
            _ => None,
        }
    }
}

/// Formato del código fuente COBOL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SourceFormat {
    /// Formato fijo tradicional (tarjeta de 80 columnas).
    Fixed,
    /// Formato libre: la línea entera es código.
    Free,
}

/// Clase de un token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    #[error("línea {line}: no se pudo leer el copybook {path:?}")]
    BadCopybook { line: u32, path: String },
    #[error("anidación de COPY excesiva (límite {limit})")]
    CopyTooDeep { limit: u32 },
    #[error("línea {line}: directiva REPLACE sin END-REPLACE")]
    ReplaceUnterminated { line: u32 },
    #[error("línea {line}: sintaxis inválida en REPLACE — {hint}")]
    BadReplaceSyntax { line: u32, hint: String },
}

/// Profundidad máxima de anidación para `COPY` recursivos.
const COPY_DEPTH_LIMIT: u32 = 16;

/// Expande las directivas `COPY '<path>'.` reemplazándolas por el
/// contenido del fichero referenciado. Acepta paths absolutos o, si se
/// indica `base_dir`, rutas relativas a ese directorio.
///
/// Honra la directiva `REPLACE`: bloque `REPLACE ==FROM== BY ==TO==
/// [...] END-REPLACE` instala reglas activas para el RESTO del archivo
/// (sustitución case-insensitive sobre tokens completos), y `REPLACE
/// OFF` las apaga. El siguiente `REPLACE` reemplaza el set activo (no
/// se acumula), siguiendo el modelo del estándar COBOL'85. Las reglas
/// son de scope del archivo: NO atraviesan a los copybooks expandidos.
pub fn preprocess(
    source: &str,
    base_dir: Option<&std::path::Path>,
) -> Result<String, LexError> {
    expand(source, base_dir, 0)
}

fn expand(
    source: &str,
    base_dir: Option<&std::path::Path>,
    depth: u32,
) -> Result<String, LexError> {
    if depth > COPY_DEPTH_LIMIT {
        return Err(LexError::CopyTooDeep {
            limit: COPY_DEPTH_LIMIT,
        });
    }
    let mut out = String::with_capacity(source.len());
    // Reglas REPLACE activas en este archivo. `Vec<(from, to)>` porque
    // el orden importa (una regla puede generar texto que matchea otra).
    let mut active_rules: Vec<(String, String)> = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let line_num = (i + 1) as u32;
        let trimmed = raw.trim_start();
        let upper = trimmed.to_uppercase();

        // `REPLACE OFF` — apaga las reglas. Aceptamos punto opcional al
        // final y cualquier cola (ej. comentarios) para no atragantarnos.
        if upper == "REPLACE OFF"
            || upper.starts_with("REPLACE OFF.")
            || upper.starts_with("REPLACE OFF ")
        {
            active_rules.clear();
            out.push_str("*> chaka: REPLACE OFF\n");
            i += 1;
            continue;
        }

        // `REPLACE ... END-REPLACE` — bloque. Acumulamos líneas hasta
        // encontrar `END-REPLACE` para soportar bloques multi-línea (lo
        // habitual en COBOL real).
        if upper.starts_with("REPLACE ") || upper == "REPLACE" || upper.starts_with("REPLACE.") {
            let mut block = String::from(trimmed);
            let start_line = line_num;
            while !contains_end_replace(&block) {
                i += 1;
                if i >= lines.len() {
                    return Err(LexError::ReplaceUnterminated { line: start_line });
                }
                block.push(' ');
                block.push_str(lines[i].trim());
            }
            // Estándar COBOL: el nuevo REPLACE reemplaza el set activo.
            active_rules = parse_replace_block(&block, start_line)?;
            out.push_str("*> chaka: REPLACE aplicado\n");
            i += 1;
            continue;
        }

        // COPY: expandir; las reglas activas NO atraviesan al copybook
        // (la directiva `COPY ... REPLACING` es OTRO mecanismo, fuera de
        // esta iteración).
        if let Some(rest) = strip_copy_prefix(&upper, trimmed) {
            let path = parse_copy_path(rest).ok_or_else(|| LexError::BadCopybook {
                line: line_num,
                path: rest.to_string(),
            })?;
            let resolved = resolve_copy_path(&path, base_dir);
            let content = std::fs::read_to_string(&resolved).map_err(|_| LexError::BadCopybook {
                line: line_num,
                path: resolved.display().to_string(),
            })?;
            let nested = expand(&content, resolved.parent(), depth + 1)?;
            out.push_str(&nested);
            if !nested.ends_with('\n') {
                out.push('\n');
            }
            i += 1;
            continue;
        }

        // Línea normal: aplicar reglas activas y emitir.
        let processed = apply_replace_rules(raw, &active_rules);
        out.push_str(&processed);
        out.push('\n');
        i += 1;
    }
    Ok(out)
}

/// `true` si el buffer ya vio el delimitador `END-REPLACE` (case
/// insensitive, robusto a sangría y al punto opcional).
fn contains_end_replace(block: &str) -> bool {
    block.to_uppercase().contains("END-REPLACE")
}

/// Parsea un bloque `REPLACE ==A== BY ==B== [==C== BY ==D==] END-REPLACE`
/// y devuelve la lista de pares `(from, to)`. El parser es deliberadamente
/// estrecho: pseudo-texts delimitadas por `==`, separador `BY` entre el
/// par, y `END-REPLACE` como terminador. Una sintaxis fuera de eso se
/// rechaza con [`LexError::BadReplaceSyntax`] en vez de adivinar.
fn parse_replace_block(block: &str, line: u32) -> Result<Vec<(String, String)>, LexError> {
    let upper = block.to_uppercase();

    // Strip "REPLACE" del inicio y "END-REPLACE..." del final.
    let after_kw = upper
        .find("REPLACE")
        .map(|p| p + "REPLACE".len())
        .unwrap_or(0);
    let end_at = upper
        .find("END-REPLACE")
        .ok_or_else(|| LexError::ReplaceUnterminated { line })?;
    if end_at < after_kw {
        return Err(LexError::BadReplaceSyntax {
            line,
            hint: "END-REPLACE aparece antes que REPLACE".into(),
        });
    }
    let body = &block[after_kw..end_at];

    // Split por `==`: el patrón esperado para N reglas es
    // `["", "FROM_1", " BY ", "TO_1", " ", "FROM_2", " BY ", "TO_2", ""]`
    // o sea 4*N + 1 fragmentos.
    let parts: Vec<&str> = body.split("==").collect();
    if parts.len() < 5 || parts.len() % 4 != 1 {
        return Err(LexError::BadReplaceSyntax {
            line,
            hint: format!(
                "esperaba pares `==FROM== BY ==TO==`, conté {} fragmentos `==`",
                parts.len()
            ),
        });
    }

    let mut rules = Vec::with_capacity(parts.len() / 4);
    let mut k = 1;
    while k + 2 < parts.len() {
        let from = parts[k].trim();
        let by = parts[k + 1].trim().to_uppercase();
        let to = parts[k + 2].trim();
        if by != "BY" {
            return Err(LexError::BadReplaceSyntax {
                line,
                hint: format!("esperaba separador `BY` entre pseudo-texts, encontré {by:?}"),
            });
        }
        if from.is_empty() {
            return Err(LexError::BadReplaceSyntax {
                line,
                hint: "pseudo-text FROM vacío".into(),
            });
        }
        rules.push((from.to_string(), to.to_string()));
        k += 4;
    }
    Ok(rules)
}

/// Aplica todas las reglas a la línea, en orden de declaración. La
/// sustitución es case-insensitive y se ata a límites de palabra
/// (alfa-numérico), así `REPLACE ==FOO== BY ==BAR==` no toca
/// `FOOBAR` ni `MYFOO` pero sí `FOO` y `Foo`.
fn apply_replace_rules(line: &str, rules: &[(String, String)]) -> String {
    if rules.is_empty() {
        return line.to_string();
    }
    let mut current = line.to_string();
    for (from, to) in rules {
        current = case_insensitive_word_replace(&current, from, to);
    }
    current
}

/// Sustitución case-insensitive de `needle` por `replacement` en
/// `haystack`, sólo cuando la ocurrencia está delimitada por caracteres
/// no alfanuméricos (o por inicio/fin de cadena). Idempotente para
/// needles vacíos (devuelve haystack tal cual).
fn case_insensitive_word_replace(haystack: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return haystack.to_string();
    }
    let hay_up = haystack.to_ascii_uppercase();
    let nee_up = needle.to_ascii_uppercase();
    let bytes = haystack.as_bytes();
    let mut result = String::with_capacity(haystack.len());
    let mut last = 0;
    let mut search = 0;
    while let Some(rel) = hay_up[search..].find(&nee_up) {
        let abs = search + rel;
        let end = abs + nee_up.len();
        let before_ok = abs == 0 || !bytes[abs - 1].is_ascii_alphanumeric();
        let after_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            result.push_str(&haystack[last..abs]);
            result.push_str(replacement);
            last = end;
            search = end;
        } else {
            // Avance mínimo seguro: 1 byte (todos los caracteres ASCII en
            // contexto COBOL caben en este avance; UTF-8 multibyte no
            // entra en identifiers, así que es seguro).
            search = abs + 1;
        }
    }
    result.push_str(&haystack[last..]);
    result
}

/// Si la línea (en mayúsculas, sin sangría) empieza por `COPY ` devuelve
/// el resto (sin la palabra `COPY` y con la sangría original conservada
/// no importa — el resto es lo que viene tras `COPY`).
fn strip_copy_prefix<'a>(upper: &str, original: &'a str) -> Option<&'a str> {
    if !upper.starts_with("COPY ") && upper != "COPY" {
        return None;
    }
    let trimmed = original.trim_start();
    let rest = trimmed.get("COPY".len()..)?.trim_start();
    Some(rest)
}

/// Extrae la ruta del copybook: `'path'`, `"path"` o un nombre desnudo
/// (sin extensión, se le agrega `.cpy`). Termina al primer `.` o `,`
/// fuera de comillas.
fn parse_copy_path(rest: &str) -> Option<String> {
    let rest = rest.trim();
    if let Some(end) = rest.strip_prefix('\'').and_then(|r| r.find('\'')) {
        return Some(rest[1..1 + end].to_string());
    }
    if let Some(end) = rest.strip_prefix('"').and_then(|r| r.find('"')) {
        return Some(rest[1..1 + end].to_string());
    }
    // Nombre desnudo: lo que va hasta el primer punto, espacio o coma.
    let stop = rest
        .find(|c: char| c == '.' || c == ',' || c.is_ascii_whitespace())
        .unwrap_or(rest.len());
    if stop == 0 {
        return None;
    }
    let name = &rest[..stop];
    if name.contains('.') {
        Some(name.to_string())
    } else {
        Some(format!("{name}.cpy"))
    }
}

/// Resuelve la ruta de un copybook contra `base_dir` si la ruta es
/// relativa; respeta los paths absolutos tal cual.
fn resolve_copy_path(path: &str, base_dir: Option<&std::path::Path>) -> std::path::PathBuf {
    let p = std::path::PathBuf::from(path);
    if p.is_absolute() {
        return p;
    }
    if let Some(base) = base_dir {
        return base.join(p);
    }
    p
}

/// Tokeniza un fuente del dialecto COBOL. Falla con el primer
/// [`LexError`]. Aplica primero el preprocesador (`COPY` / `REPLACE`)
/// sin directorio base — los `COPY` deben usar rutas absolutas.
pub fn lex(source: &str, format: SourceFormat) -> Result<Vec<Token>, LexError> {
    lex_with_dialect(source, format, Dialect::Cobol, None)
}

/// Versión de [`lex`] que acepta un `base_dir` para resolver rutas
/// relativas en las directivas `COPY`. Equivale a
/// [`lex_with_dialect`] con `Dialect::Cobol`.
pub fn lex_with_base(
    source: &str,
    format: SourceFormat,
    base_dir: Option<&std::path::Path>,
) -> Result<Vec<Token>, LexError> {
    lex_with_dialect(source, format, Dialect::Cobol, base_dir)
}

/// Forma general del lexer: despacha por dialecto y resuelve `COPY`
/// contra `base_dir` si la ruta es relativa. Hoy todo `Dialect` se
/// resuelve por la misma ruta (sólo `Cobol` está implementado).
pub fn lex_with_dialect(
    source: &str,
    format: SourceFormat,
    dialect: Dialect,
    base_dir: Option<&std::path::Path>,
) -> Result<Vec<Token>, LexError> {
    match dialect {
        Dialect::Cobol => lex_cobol(source, format, base_dir),
    }
}

/// Lexa un fuente del dialecto COBOL (la implementación efectiva).
fn lex_cobol(
    source: &str,
    format: SourceFormat,
    base_dir: Option<&std::path::Path>,
) -> Result<Vec<Token>, LexError> {
    let expanded = preprocess(source, base_dir)?;
    let mut tokens = Vec::new();
    for (idx, raw) in expanded.lines().enumerate() {
        let line = (idx + 1) as u32;
        if let Some((content, base_col)) = prepare_line(raw, format) {
            lex_line(&content, line, base_col, &mut tokens)?;
        }
    }
    Ok(tokens)
}

/// Extrae el área de código de una línea según el format. `None` si la
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
            // El separador de sentencia COBOL siempre lleva un espacio
            // (o el fin de línea) detrás. Un punto pegado a un carácter
            // —`ZZ9.99`— no es separador: pertenece a una PICTURE de
            // edición y se emite como símbolo para que el parser lo
            // reensamble dentro de la cláusula.
            let is_separator = chars
                .get(i + 1)
                .map_or(true, |n| n.is_whitespace());
            out.push(Token {
                kind: if is_separator {
                    TokenKind::Period
                } else {
                    TokenKind::Symbol
                },
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
    fn dialect_from_extension_recognizes_cobol_suffixes() {
        assert_eq!(Dialect::from_extension("cob"), Some(Dialect::Cobol));
        assert_eq!(Dialect::from_extension("CBL"), Some(Dialect::Cobol));
        assert_eq!(Dialect::from_extension("cpy"), Some(Dialect::Cobol));
        assert_eq!(Dialect::from_extension("rs"), None);
    }

    #[test]
    fn lex_with_dialect_cobol_matches_default_lex() {
        let src = "PROCEDURE DIVISION.\nMAIN.\n DISPLAY 'OK'.\n";
        let a = lex(src, SourceFormat::Free).unwrap();
        let b = lex_with_dialect(src, SourceFormat::Free, Dialect::Cobol, None).unwrap();
        assert_eq!(a, b);
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
    fn period_inside_an_edit_picture_is_not_a_separator() {
        // El punto de `ZZ9.99` va pegado a un dígito: es símbolo, no
        // terminador. El punto final, con espacio detrás, sí termina.
        let toks = kinds("PIC Z,ZZ9.99 .", SourceFormat::Free);
        let dots: Vec<TokenKind> = toks
            .iter()
            .filter(|(_, t)| t == ".")
            .map(|(k, _)| *k)
            .collect();
        assert_eq!(dots, vec![TokenKind::Symbol, TokenKind::Period]);
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

    // -----------------------------------------------------------------
    //  REPLACE — preprocesador
    // -----------------------------------------------------------------

    fn pp(src: &str) -> String {
        preprocess(src, None).expect("preprocess OK")
    }

    #[test]
    fn replace_simple_aplica_una_regla() {
        let src = "\
REPLACE ==FOO== BY ==BAR== END-REPLACE.
DISPLAY FOO.
DISPLAY foo.";
        let out = pp(src);
        assert!(out.contains("DISPLAY BAR."));
        assert!(out.contains("DISPLAY BAR."), "case insensitive: {out:?}");
    }

    #[test]
    fn replace_respeta_limites_de_palabra() {
        let src = "\
REPLACE ==FOO== BY ==BAR== END-REPLACE.
DISPLAY FOOBAR.
DISPLAY MYFOO.
DISPLAY FOO-BAZ.";
        let out = pp(src);
        // No tocar substrings dentro de palabras.
        assert!(out.contains("DISPLAY FOOBAR."), "got: {out}");
        assert!(out.contains("DISPLAY MYFOO."), "got: {out}");
        // El guión separa identifiers en COBOL, así que sí se reemplaza.
        assert!(out.contains("DISPLAY BAR-BAZ."), "got: {out}");
    }

    #[test]
    fn replace_off_apaga_las_reglas() {
        let src = "\
REPLACE ==FOO== BY ==BAR== END-REPLACE.
DISPLAY FOO.
REPLACE OFF.
DISPLAY FOO.";
        let out = pp(src);
        // Antes del OFF se reemplaza; después, no.
        let mut lines = out.lines().filter(|l| l.starts_with("DISPLAY"));
        assert_eq!(lines.next(), Some("DISPLAY BAR."));
        assert_eq!(lines.next(), Some("DISPLAY FOO."));
    }

    #[test]
    fn replace_multiples_reglas_en_un_bloque() {
        let src = "\
REPLACE ==FOO== BY ==BAR== ==BAZ== BY ==QUX== END-REPLACE.
DISPLAY FOO BAZ.";
        let out = pp(src);
        assert!(out.contains("DISPLAY BAR QUX."), "got: {out}");
    }

    #[test]
    fn replace_multilinea_se_acumula_hasta_end_replace() {
        let src = "\
REPLACE
  ==FOO== BY ==BAR==
  ==BAZ== BY ==QUX==
END-REPLACE.
DISPLAY FOO BAZ.";
        let out = pp(src);
        assert!(out.contains("DISPLAY BAR QUX."), "got: {out}");
    }

    #[test]
    fn replace_sucesivo_reemplaza_el_set_activo() {
        // Segundo REPLACE pisa las reglas del primero (no se acumula).
        let src = "\
REPLACE ==FOO== BY ==BAR== END-REPLACE.
DISPLAY FOO.
REPLACE ==BAZ== BY ==QUX== END-REPLACE.
DISPLAY FOO.
DISPLAY BAZ.";
        let out = pp(src);
        let mut lines = out.lines().filter(|l| l.starts_with("DISPLAY"));
        assert_eq!(lines.next(), Some("DISPLAY BAR."));
        assert_eq!(
            lines.next(),
            Some("DISPLAY FOO."),
            "tras el 2do REPLACE, FOO no debería reemplazarse"
        );
        assert_eq!(lines.next(), Some("DISPLAY QUX."));
    }

    #[test]
    fn replace_no_atraviesa_copy_expandido() {
        use std::io::Write;
        // Copybook con un identifier que el padre intenta reescribir;
        // por scope, NO debe tocarse.
        let dir = tempfile::tempdir().unwrap();
        let cpy = dir.path().join("aux.cpy");
        std::fs::File::create(&cpy)
            .unwrap()
            .write_all(b"DISPLAY FOO.")
            .unwrap();
        let src = format!(
            "REPLACE ==FOO== BY ==BAR== END-REPLACE.\nDISPLAY FOO.\nCOPY '{}'.\n",
            cpy.display()
        );
        let out = preprocess(&src, Some(dir.path())).unwrap();
        let mut lines = out.lines().filter(|l| l.starts_with("DISPLAY"));
        assert_eq!(lines.next(), Some("DISPLAY BAR."));
        assert_eq!(
            lines.next(),
            Some("DISPLAY FOO."),
            "dentro del copybook FOO no debe reescribirse: {out}",
        );
    }

    #[test]
    fn replace_sin_end_replace_da_error() {
        let src = "\
REPLACE ==FOO== BY ==BAR==
DISPLAY FOO.";
        let err = preprocess(src, None).unwrap_err();
        match err {
            LexError::ReplaceUnterminated { line } => assert_eq!(line, 1),
            other => panic!("esperaba ReplaceUnterminated, fue {other:?}"),
        }
    }

    #[test]
    fn replace_sintaxis_mala_da_error() {
        let src = "REPLACE ==FOO== INTO ==BAR== END-REPLACE.";
        let err = preprocess(src, None).unwrap_err();
        assert!(matches!(err, LexError::BadReplaceSyntax { .. }));
    }
}
