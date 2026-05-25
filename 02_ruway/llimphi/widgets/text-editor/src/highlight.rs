//! Syntax highlighting. Cada `Language` produce una `Vec<LineSpans>`:
//! por línea, una secuencia ordenada de `(start_col, end_col, TokenKind)`
//! que cubre toda la línea. El renderer pinta cada span con el color
//! que el [`SyntaxPalette`] mapea desde el `TokenKind`.
//!
//! - **Rust / Python**: tree-sitter parseando el buffer entero (ineficiente
//!   pero adecuado para celdas de notebook ≤ ~1k LOC). Las queries se
//!   compilan una vez por `Language`.
//! - **WAT**: tokenizer en Rust puro (LISP-like: paren, `$`-prefijo,
//!   strings, números, keywords típicos del subset MVP).
//! - **Plain**: un solo span por línea con `TokenKind::Other`.

use llimphi_ui::llimphi_raster::peniko::Color;

/// Lenguajes soportados — la matriz se extiende sumando un variant +
/// una rama en [`Highlighter::tokenize_line`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Plain,
    Rust,
    Python,
    Wat,
}

impl Language {
    /// Heurística: derivar el `Language` del `language` del `CellKind`.
    pub fn from_cell_language(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "rust" | "rs" => Language::Rust,
            "python" | "py" => Language::Python,
            "wasm" | "wat" => Language::Wat,
            _ => Language::Plain,
        }
    }
}

/// Categorías de token — lo suficientemente granular para colores
/// distintos sin saturar el theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    Type,
    Function,
    String,
    Number,
    Comment,
    Operator,
    Punctuation,
    Identifier,
    Other,
}

/// Un span dentro de una línea: `[start_col..end_col)` de la línea,
/// más su categoría.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start_col: usize,
    pub end_col: usize,
    pub kind: TokenKind,
}

/// Paleta de colores por categoría — el theme la deriva.
#[derive(Debug, Clone, Copy)]
pub struct SyntaxPalette {
    pub keyword: Color,
    pub typ: Color,
    pub function: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    pub operator: Color,
    pub punctuation: Color,
    pub identifier: Color,
    pub other: Color,
}

impl SyntaxPalette {
    pub fn color(&self, k: TokenKind) -> Color {
        match k {
            TokenKind::Keyword => self.keyword,
            TokenKind::Type => self.typ,
            TokenKind::Function => self.function,
            TokenKind::String => self.string,
            TokenKind::Number => self.number,
            TokenKind::Comment => self.comment,
            TokenKind::Operator => self.operator,
            TokenKind::Punctuation => self.punctuation,
            TokenKind::Identifier => self.identifier,
            TokenKind::Other => self.other,
        }
    }

    /// Paleta dark sobria — derivada del `Theme::dark` + colores
    /// hardcoded para las categorías que el theme no tiene como
    /// semánticas (string, number, comment, keyword).
    pub fn dark_default(theme: &llimphi_theme::Theme) -> Self {
        // Helper: color rgb opaco.
        fn rgb(r: u8, g: u8, b: u8) -> Color {
            Color::from_rgb8(r, g, b)
        }
        Self {
            keyword: rgb(198, 120, 221),     // morado: keywords
            typ: rgb(229, 192, 123),         // amarillo cálido: tipos
            function: rgb(97, 175, 239),     // azul: funciones
            string: rgb(152, 195, 121),      // verde: strings
            number: rgb(209, 154, 102),      // naranja: números
            comment: theme.fg_muted,          // muted: comentarios
            operator: theme.fg_text,
            punctuation: theme.fg_muted,
            identifier: theme.fg_text,
            other: theme.fg_text,
        }
    }
}

// Pool thread-local de parsers tree-sitter. Reconstruir el parser
// (con `set_language`) es caro; reusarlo entre highlights del mismo
// lenguaje es un ahorro grande. `tree_sitter::Parser` no es Send/
// Sync ni Clone, así que vive en thread-local — un parser por
// lenguaje por thread.
thread_local! {
    static PARSER_POOL: std::cell::RefCell<std::collections::HashMap<Language, tree_sitter::Parser>>
        = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Highlighter — fina capa sin estado mutable propio. La parser real
/// vive en el pool thread-local.
pub struct Highlighter {
    language: Language,
}

impl Highlighter {
    pub fn new(language: Language) -> Self {
        Self { language }
    }

    pub fn language(&self) -> Language {
        self.language
    }

    pub fn set_language(&mut self, language: Language) {
        self.language = language;
    }

    /// Tokeniza el `source` entero y devuelve los spans por línea.
    /// `result.len() == source.lines().count().max(1)`.
    pub fn highlight(&mut self, source: &str) -> Vec<Vec<Span>> {
        match self.language {
            Language::Plain => plain_lines(source),
            Language::Wat => highlight_wat(source),
            Language::Rust => self.highlight_treesitter(source, rust_kind),
            Language::Python => self.highlight_treesitter(source, python_kind),
        }
    }

    fn highlight_treesitter(
        &mut self,
        source: &str,
        kind_of: fn(&str) -> Option<TokenKind>,
    ) -> Vec<Vec<Span>> {
        // Toma el parser del pool (o lo crea); parsea; guarda devuelta.
        // Si make_ts_parser falla (lenguaje sin grammar), fallback Plain.
        let language = self.language;
        let tree_opt = PARSER_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            let parser = pool.entry(language).or_insert_with(|| {
                make_ts_parser(language).unwrap_or_else(tree_sitter::Parser::new)
            });
            parser.parse(source, None)
        });
        let Some(tree) = tree_opt else {
            return plain_lines(source);
        };

        // Por línea: recopilamos spans de los nodos *named* tipados que
        // matchean kind_of. Luego rellenamos los huecos con `Other`.
        let line_count = source.lines().count().max(1)
            + (if source.ends_with('\n') { 1 } else { 0 });
        let mut per_line: Vec<Vec<Span>> = vec![Vec::new(); line_count.max(1)];

        let mut stack: Vec<tree_sitter::Node> = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.child_count() == 0 {
                // hoja: tomamos el tipo del nodo (token).
                let kind = node.kind();
                if let Some(tk) = kind_of(kind) {
                    let start = node.start_position();
                    let end = node.end_position();
                    // Sólo manejamos tokens single-line (los multi-line
                    // como block strings se splitean por línea).
                    if start.row == end.row {
                        if let Some(line) = per_line.get_mut(start.row) {
                            line.push(Span {
                                start_col: start.column,
                                end_col: end.column,
                                kind: tk,
                            });
                        }
                    } else {
                        // Multi-line: marca cada línea entera como ese kind.
                        // Aproximación; suficiente para strings multi-línea.
                        for row in start.row..=end.row {
                            if let Some(line) = per_line.get_mut(row) {
                                let line_text =
                                    source.lines().nth(row).unwrap_or("");
                                let s = if row == start.row { start.column } else { 0 };
                                let e =
                                    if row == end.row { end.column } else { line_text.chars().count() };
                                line.push(Span { start_col: s, end_col: e, kind: tk });
                            }
                        }
                    }
                }
            } else {
                for i in (0..node.child_count()).rev() {
                    if let Some(c) = node.child(i) {
                        stack.push(c);
                    }
                }
            }
        }

        // Por cada línea: ordena, fusiona overlapping, rellena huecos.
        let mut result: Vec<Vec<Span>> = Vec::with_capacity(per_line.len());
        for (row, mut spans) in per_line.into_iter().enumerate() {
            let line_text = source.lines().nth(row).unwrap_or("");
            spans.sort_by_key(|s| s.start_col);
            result.push(fill_gaps(spans, line_text.chars().count()));
        }
        result
    }
}

fn make_ts_parser(language: Language) -> Option<tree_sitter::Parser> {
    let mut parser = tree_sitter::Parser::new();
    let lang: tree_sitter::Language = match language {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        _ => return None,
    };
    parser.set_language(&lang).ok()?;
    Some(parser)
}

/// Mapeo de tree-sitter node `kind` → TokenKind para Rust.
fn rust_kind(kind: &str) -> Option<TokenKind> {
    // Lista deliberadamente acotada al subset común; nodos no listados
    // caen como Identifier/Other vía fill_gaps.
    match kind {
        // Keywords
        "fn" | "let" | "mut" | "const" | "static" | "if" | "else" | "match"
        | "for" | "while" | "loop" | "break" | "continue" | "return" | "use"
        | "mod" | "pub" | "impl" | "trait" | "struct" | "enum" | "type"
        | "where" | "as" | "in" | "ref" | "move" | "self" | "Self" | "crate"
        | "super" | "async" | "await" | "dyn" | "unsafe" | "extern" => {
            Some(TokenKind::Keyword)
        }
        // Tipos primitivos
        "primitive_type" => Some(TokenKind::Type),
        // Literales
        "string_literal" | "raw_string_literal" | "char_literal" | "string_content" => {
            Some(TokenKind::String)
        }
        "integer_literal" | "float_literal" | "boolean_literal" => Some(TokenKind::Number),
        // Comentarios
        "line_comment" | "block_comment" => Some(TokenKind::Comment),
        _ => None,
    }
}

/// Mapeo para Python.
fn python_kind(kind: &str) -> Option<TokenKind> {
    match kind {
        "def" | "class" | "if" | "elif" | "else" | "for" | "while" | "return"
        | "import" | "from" | "as" | "in" | "is" | "not" | "and" | "or"
        | "with" | "try" | "except" | "finally" | "raise" | "yield" | "pass"
        | "break" | "continue" | "global" | "nonlocal" | "lambda" | "True"
        | "False" | "None" | "async" | "await" => Some(TokenKind::Keyword),
        "string" | "string_start" | "string_content" | "string_end" => Some(TokenKind::String),
        "integer" | "float" | "true" | "false" | "none" => Some(TokenKind::Number),
        "comment" => Some(TokenKind::Comment),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// WAT — tokenizer en Rust puro (sin tree-sitter).
// ---------------------------------------------------------------------

fn highlight_wat(source: &str) -> Vec<Vec<Span>> {
    let mut out: Vec<Vec<Span>> = Vec::new();
    for line in iterate_lines(source) {
        out.push(tokenize_wat_line(line));
    }
    out
}

fn iterate_lines(source: &str) -> Vec<&str> {
    let mut out: Vec<&str> = source.lines().collect();
    if source.ends_with('\n') || source.is_empty() {
        out.push("");
    }
    if out.is_empty() {
        out.push("");
    }
    out
}

fn tokenize_wat_line(line: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut i = 0usize;

    while i < len {
        let c = chars[i];

        if c.is_whitespace() {
            let start = i;
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            out.push(Span { start_col: start, end_col: i, kind: TokenKind::Other });
            continue;
        }

        // Comentario línea `;; ...`
        if c == ';' && i + 1 < len && chars[i + 1] == ';' {
            out.push(Span { start_col: i, end_col: len, kind: TokenKind::Comment });
            break;
        }

        // Paren
        if c == '(' || c == ')' {
            out.push(Span { start_col: i, end_col: i + 1, kind: TokenKind::Punctuation });
            i += 1;
            continue;
        }

        // String "..."
        if c == '"' {
            let start = i;
            i += 1;
            while i < len {
                let cc = chars[i];
                if cc == '\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if cc == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(Span { start_col: start, end_col: i, kind: TokenKind::String });
            continue;
        }

        // Identificador `$nombre`
        if c == '$' {
            let start = i;
            i += 1;
            while i < len && is_wat_ident_char(chars[i]) {
                i += 1;
            }
            out.push(Span { start_col: start, end_col: i, kind: TokenKind::Identifier });
            continue;
        }

        // Número (entero/hex/float — simplificado: empieza con dígito o -dígito).
        if c.is_ascii_digit() || (c == '-' && i + 1 < len && chars[i + 1].is_ascii_digit()) {
            let start = i;
            if c == '-' {
                i += 1;
            }
            while i < len {
                let cc = chars[i];
                if cc.is_ascii_digit() || cc == '.' || cc == 'x' || cc.is_ascii_hexdigit() {
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Span { start_col: start, end_col: i, kind: TokenKind::Number });
            continue;
        }

        // Word: keyword o identificador
        if is_wat_word_start(c) {
            let start = i;
            while i < len && is_wat_ident_char(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let kind = wat_word_kind(&word);
            out.push(Span { start_col: start, end_col: i, kind });
            continue;
        }

        // Otros (operadores como `.`)
        out.push(Span { start_col: i, end_col: i + 1, kind: TokenKind::Operator });
        i += 1;
    }

    fill_gaps(out, len)
}

fn is_wat_word_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}
fn is_wat_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '-' | '/' | ':' | '<' | '=' | '>' | '?' | '@' | '\\' | '^' | '`' | '|' | '~')
}

fn wat_word_kind(w: &str) -> TokenKind {
    const KEYWORDS: &[&str] = &[
        "module", "func", "param", "result", "local", "import", "export",
        "memory", "data", "table", "elem", "type", "global", "start", "block",
        "loop", "if", "then", "else", "end", "br", "br_if", "br_table",
        "return", "call", "call_indirect",
    ];
    const TYPES: &[&str] = &["i32", "i64", "f32", "f64", "v128", "funcref", "externref", "anyref"];

    if KEYWORDS.contains(&w) {
        TokenKind::Keyword
    } else if TYPES.contains(&w) {
        TokenKind::Type
    } else if w.contains('.') {
        // Instrucciones tipo `i32.const`, `local.get`, etc.
        TokenKind::Function
    } else {
        TokenKind::Identifier
    }
}

// ---------------------------------------------------------------------
// Plain + utilities
// ---------------------------------------------------------------------

fn plain_lines(source: &str) -> Vec<Vec<Span>> {
    let mut out: Vec<Vec<Span>> = Vec::new();
    for line in iterate_lines(source) {
        let len = line.chars().count();
        out.push(vec![Span { start_col: 0, end_col: len, kind: TokenKind::Other }]);
    }
    out
}

/// Rellena los huecos entre spans con `Other` para cubrir `[0..line_len)`.
fn fill_gaps(spans: Vec<Span>, line_len: usize) -> Vec<Span> {
    if spans.is_empty() {
        return vec![Span { start_col: 0, end_col: line_len, kind: TokenKind::Other }];
    }
    let mut out: Vec<Span> = Vec::with_capacity(spans.len() * 2);
    let mut cursor = 0usize;
    for s in spans {
        if s.start_col > cursor {
            out.push(Span { start_col: cursor, end_col: s.start_col, kind: TokenKind::Other });
        }
        // Clampea overlaps con el anterior.
        if s.end_col > cursor {
            let start_col = s.start_col.max(cursor);
            out.push(Span { start_col, end_col: s.end_col, kind: s.kind });
            cursor = s.end_col;
        }
    }
    if cursor < line_len {
        out.push(Span { start_col: cursor, end_col: line_len, kind: TokenKind::Other });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_devuelve_un_span_por_linea() {
        let mut h = Highlighter::new(Language::Plain);
        let r = h.highlight("hola\nmundo");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].len(), 1);
        assert_eq!(r[0][0].kind, TokenKind::Other);
    }

    #[test]
    fn wat_paren_es_punctuation() {
        let mut h = Highlighter::new(Language::Wat);
        let r = h.highlight("(module)");
        let line = &r[0];
        let paren = line.iter().find(|s| s.kind == TokenKind::Punctuation).unwrap();
        assert_eq!(paren.start_col, 0);
        assert_eq!(paren.end_col, 1);
    }

    #[test]
    fn wat_keyword_module_clasifica_como_keyword() {
        let mut h = Highlighter::new(Language::Wat);
        let r = h.highlight("(module)");
        let kw = r[0].iter().find(|s| s.kind == TokenKind::Keyword).unwrap();
        assert_eq!(kw.start_col, 1);
        assert_eq!(kw.end_col, 7);
    }

    #[test]
    fn wat_tipo_i32_es_type() {
        let mut h = Highlighter::new(Language::Wat);
        let r = h.highlight("(result i32)");
        assert!(r[0].iter().any(|s| s.kind == TokenKind::Type));
    }

    #[test]
    fn wat_string_y_comment() {
        let mut h = Highlighter::new(Language::Wat);
        let r = h.highlight(r#"(data "hola") ;; comentario"#);
        assert!(r[0].iter().any(|s| s.kind == TokenKind::String));
        assert!(r[0].iter().any(|s| s.kind == TokenKind::Comment));
    }

    #[test]
    fn wat_instruction_dotted_es_function() {
        let mut h = Highlighter::new(Language::Wat);
        let r = h.highlight("i32.const 42");
        assert!(r[0].iter().any(|s| s.kind == TokenKind::Function));
        assert!(r[0].iter().any(|s| s.kind == TokenKind::Number));
    }

    #[test]
    fn rust_keyword_fn() {
        let mut h = Highlighter::new(Language::Rust);
        let r = h.highlight("fn main() {}");
        // El span de "fn" debe estar marcado como keyword.
        assert!(r[0].iter().any(|s| s.kind == TokenKind::Keyword));
    }

    #[test]
    fn python_keyword_def() {
        let mut h = Highlighter::new(Language::Python);
        let r = h.highlight("def f():\n    return 1");
        assert!(r[0].iter().any(|s| s.kind == TokenKind::Keyword));
        // "return" en la línea 2.
        assert!(r[1].iter().any(|s| s.kind == TokenKind::Keyword));
    }

    #[test]
    fn fill_gaps_rellena_y_clampea() {
        let spans = vec![
            Span { start_col: 2, end_col: 4, kind: TokenKind::Keyword },
            Span { start_col: 6, end_col: 9, kind: TokenKind::String },
        ];
        let filled = fill_gaps(spans, 10);
        // [Other 0..2] [Keyword 2..4] [Other 4..6] [String 6..9] [Other 9..10]
        assert_eq!(filled.len(), 5);
        assert_eq!(filled[0].kind, TokenKind::Other);
        assert_eq!(filled[4].kind, TokenKind::Other);
    }

    #[test]
    fn from_cell_language_mapea_aliases() {
        assert_eq!(Language::from_cell_language("rust"), Language::Rust);
        assert_eq!(Language::from_cell_language("rs"), Language::Rust);
        assert_eq!(Language::from_cell_language("py"), Language::Python);
        assert_eq!(Language::from_cell_language("wat"), Language::Wat);
        assert_eq!(Language::from_cell_language("desconocido"), Language::Plain);
    }
}
