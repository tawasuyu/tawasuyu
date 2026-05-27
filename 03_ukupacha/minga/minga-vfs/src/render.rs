//! Renderizado de un `SemanticNode` a texto legible. Lógica **pura**:
//! sin IO, sin FUSE, sin `sled`. El VFS la usa para materializar el
//! contenido de cada archivo virtual bajo demanda, pero es reutilizable
//! por cualquier frontend (web, TUI).
//!
//! Dos vistas complementarias:
//!
//! - [`render_source`]: reconstrucción **canónica** del código fuente.
//!   El AST semántico descartó whitespace y comentarios al ingerir
//!   (son `extra` en tree-sitter), así que esto NO recupera el archivo
//!   original byte-a-byte: es una forma *normalizada*, re-indentada por
//!   estructura. Para lenguajes con bloques por llaves (Rust/TS/JS/Go)
//!   indenta por `{`/`}`; para Python (estructura por indentación)
//!   detecta el root `module` y recurre por `function_definition`,
//!   `if_statement`, `for_statement`, etc. con indent explícito.
//!
//! - [`render_sexp`]: el árbol como S-expression indentada. Vista
//!   exacta y sin pérdida de lo que el store guarda de verdad.

use minga_core::SemanticNode;

/// Reconstruye el código fuente de un subárbol en forma canónica.
///
/// Si el root es un `module` (Python tree-sitter), delega al renderer
/// indent-aware. Si no, usa el pretty-printer general por tokens con
/// llaves.
pub fn render_source(node: &SemanticNode) -> String {
    if node.kind == "module" {
        return render_python(node);
    }
    let mut tokens = Vec::new();
    collect_leaves(node, &mut tokens);
    pretty_print(&tokens)
}

// ─── Render Python (indent-aware) ──────────────────────────────────

/// Reconstruye un archivo Python. El root es `module`; sus children son
/// statements de nivel superior. Para cada statement compuesto
/// (función, clase, if, for, while, with, try) recurre por el `block`
/// del cuerpo aumentando la indentación.
fn render_python(module: &SemanticNode) -> String {
    let mut out = String::new();
    render_py_block_children(&module.children, 0, &mut out);
    while out.ends_with([' ', '\t', '\n']) {
        out.pop();
    }
    out.push('\n');
    out
}

fn render_py_block(block: &SemanticNode, indent: usize, out: &mut String) {
    render_py_block_children(&block.children, indent, out);
}

fn render_py_block_children(children: &[SemanticNode], indent: usize, out: &mut String) {
    for child in children {
        if is_py_compound(child) {
            render_py_compound(child, indent, out);
        } else if child.kind == ":" || child.kind == "comment" {
            // tokens sueltos del header de un parent ya capturado; ignorar
            // si llegan a este nivel (no debería pasar con root `module`).
            continue;
        } else {
            let mut tokens = Vec::new();
            collect_leaves(child, &mut tokens);
            if tokens.is_empty() {
                continue;
            }
            push_indent(out, indent);
            join_py_tokens(&tokens, out);
            out.push('\n');
        }
    }
}

fn is_py_compound(node: &SemanticNode) -> bool {
    matches!(
        node.kind.as_str(),
        "function_definition"
            | "async_function_definition"
            | "class_definition"
            | "decorated_definition"
            | "if_statement"
            | "for_statement"
            | "async_for_statement"
            | "while_statement"
            | "with_statement"
            | "async_with_statement"
            | "try_statement"
            | "match_statement"
    )
}

/// Renderiza un statement compuesto. Itera los children: los tokens
/// previos a un `block` forman el header (terminado en `:`); cada `block`
/// se renderiza con indent+1; las cláusulas anidadas (`elif_clause`,
/// `else_clause`, `except_clause`, `finally_clause`, `case_clause`) se
/// recursan como compounds en el mismo nivel de indent.
fn render_py_compound(node: &SemanticNode, indent: usize, out: &mut String) {
    let mut header: Vec<String> = Vec::new();
    let mut header_emitted = false;
    for c in &node.children {
        match c.kind.as_str() {
            "block" => {
                if !header_emitted {
                    flush_py_header(&mut header, indent, out);
                    header_emitted = true;
                }
                render_py_block(c, indent + 1, out);
            }
            "elif_clause" | "else_clause" | "except_clause" | "finally_clause"
            | "case_clause" => {
                if !header_emitted {
                    flush_py_header(&mut header, indent, out);
                    header_emitted = true;
                }
                render_py_compound(c, indent, out);
            }
            _ => collect_leaves(c, &mut header),
        }
    }
    if !header_emitted {
        // Compound sin cuerpo (raro en código real, pero posible).
        flush_py_header(&mut header, indent, out);
    }
}

fn flush_py_header(header: &mut Vec<String>, indent: usize, out: &mut String) {
    if header.is_empty() {
        return;
    }
    push_indent(out, indent);
    join_py_tokens(header, out);
    if !out.ends_with(':') {
        out.push(':');
    }
    out.push('\n');
    header.clear();
}

/// Junta tokens Python en una línea respetando las reglas de espacio
/// (compartidas con el renderer general).
fn join_py_tokens(tokens: &[String], out: &mut String) {
    for (i, tok) in tokens.iter().enumerate() {
        if i > 0 {
            let prev = tokens[i - 1].as_str();
            if needs_space(tok, prev) {
                out.push(' ');
            }
        }
        out.push_str(tok);
    }
}

/// Recolecta el texto de los nodos hoja en orden de recorrido (DFS).
/// Sólo las hojas tienen `leaf_text`; los nodos internos se recurren.
fn collect_leaves(node: &SemanticNode, out: &mut Vec<String>) {
    match &node.leaf_text {
        Some(bytes) => {
            let text = String::from_utf8_lossy(bytes);
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        }
        None => {
            for child in &node.children {
                collect_leaves(child, out);
            }
        }
    }
}

/// Tokens que no quieren un espacio a su izquierda (puntuación que se
/// pega al token anterior). Incluye `(` y `[`: en una vista normalizada
/// se pegan al identificador previo (`main()`, `v[0]`) — el caso de
/// llamada/indexado es el dominante.
fn no_space_before(t: &str) -> bool {
    matches!(
        t,
        ")" | "]" | "," | ";" | "." | "::" | "?" | ":" | "!" | "(" | "["
    )
}

/// Tokens tras los cuales no va espacio (abren un grupo o son prefijos
/// que se pegan al token siguiente).
fn no_space_after(t: &str) -> bool {
    matches!(t, "(" | "[" | "." | "::" | "!" | "#")
}

/// ¿Hace falta un espacio entre `prev` y `cur` en una misma línea?
fn needs_space(cur: &str, prev: &str) -> bool {
    !no_space_before(cur) && !no_space_after(prev)
}

fn push_indent(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("    ");
    }
}

/// Re-imprime una secuencia de tokens con indentación por llaves.
fn pretty_print(tokens: &[String]) -> String {
    let mut out = String::new();
    let mut indent: usize = 0;
    // ¿Hay ya contenido en la línea en curso?
    let mut line_open = false;

    for (i, tok) in tokens.iter().enumerate() {
        let t = tok.as_str();
        let next = tokens.get(i + 1).map(String::as_str);
        match t {
            "{" => {
                if line_open {
                    out.push(' ');
                }
                out.push('{');
                indent += 1;
                out.push('\n');
                line_open = false;
            }
            "}" => {
                indent = indent.saturating_sub(1);
                if line_open {
                    out.push('\n');
                }
                push_indent(&mut out, indent);
                out.push('}');
                line_open = true;
                // `} else`, `},`, `};`, `})`, `}.` se quedan en línea;
                // cualquier otra cosa abre línea nueva.
                if !matches!(next, Some("else") | Some(",") | Some(";") | Some(")") | Some(".")) {
                    out.push('\n');
                    line_open = false;
                }
            }
            ";" => {
                out.push(';');
                out.push('\n');
                line_open = false;
            }
            _ => {
                if !line_open {
                    push_indent(&mut out, indent);
                    line_open = true;
                } else if let Some(prev) = tokens.get(i.wrapping_sub(1)).map(String::as_str) {
                    if i > 0 && needs_space(t, prev) {
                        out.push(' ');
                    }
                }
                out.push_str(t);
            }
        }
    }

    // Final canónico: exactamente un newline.
    while out.ends_with([' ', '\t', '\n']) {
        out.pop();
    }
    out.push('\n');
    out
}

/// Renderiza el subárbol como S-expression indentada (2 espacios por
/// nivel). Cada nodo es `(kind ...)`; los nodos con `field_name` lo
/// prefijan como `field: (kind ...)`; las hojas llevan su texto entre
/// comillas. Es la representación literal de lo que hay en el store.
pub fn render_sexp(node: &SemanticNode) -> String {
    let mut out = String::new();
    sexp_node(node, 0, &mut out);
    out.push('\n');
    out
}

fn sexp_node(node: &SemanticNode, depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str("  ");
    }
    // Convención tree-sitter: el nombre de campo va FUERA del paréntesis.
    if let Some(field) = &node.field_name {
        out.push_str(field);
        out.push_str(": ");
    }
    out.push('(');
    out.push_str(&node.kind);

    match &node.leaf_text {
        Some(bytes) => {
            out.push(' ');
            out.push('"');
            out.push_str(&escape(&String::from_utf8_lossy(bytes)));
            out.push('"');
            out.push(')');
        }
        None if node.children.is_empty() => {
            out.push(')');
        }
        None => {
            for child in &node.children {
                out.push('\n');
                sexp_node(child, depth + 1, out);
            }
            out.push(')');
        }
    }
}

/// Escapa una cadena para que quepa entre comillas en la S-expression.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use minga_core::ast::SemanticNode;

    fn leaf(kind: &str, text: &str) -> SemanticNode {
        SemanticNode {
            kind: kind.to_string(),
            field_name: None,
            leaf_text: Some(text.as_bytes().to_vec()),
            children: Vec::new(),
        }
    }

    fn branch(kind: &str, children: Vec<SemanticNode>) -> SemanticNode {
        SemanticNode {
            kind: kind.to_string(),
            field_name: None,
            leaf_text: None,
            children,
        }
    }

    #[test]
    fn source_indents_on_braces() {
        // tokens: fn main ( ) { let x = 1 ; }
        let tree = branch(
            "fn_item",
            vec![
                leaf("fn", "fn"),
                leaf("ident", "main"),
                leaf("(", "("),
                leaf(")", ")"),
                leaf("{", "{"),
                leaf("let", "let"),
                leaf("ident", "x"),
                leaf("=", "="),
                leaf("int", "1"),
                leaf(";", ";"),
                leaf("}", "}"),
            ],
        );
        let out = render_source(&tree);
        assert!(out.contains("fn main()"), "tokens pegados a paréntesis: {out:?}");
        assert!(out.contains("    let x = 1;"), "cuerpo indentado: {out:?}");
        assert!(out.ends_with("}\n"), "termina en una sola llave + newline: {out:?}");
    }

    #[test]
    fn sexp_shows_kinds_fields_and_leaves() {
        let mut id = leaf("identifier", "x");
        id.field_name = Some("name".to_string());
        let tree = branch("declaration", vec![id]);
        let out = render_sexp(&tree);
        assert!(out.contains("(declaration"));
        assert!(out.contains("name: (identifier \"x\")"));
    }

    #[test]
    fn sexp_escapes_special_chars() {
        let out = render_sexp(&leaf("string", "a\"b\nc"));
        assert!(out.contains("\\\""), "comilla escapada: {out:?}");
        assert!(out.contains("\\n"), "newline escapado: {out:?}");
    }

    /// El render Python real opera sobre el árbol que produce
    /// tree-sitter al parsear. Estos tests usan ese path end-to-end —
    /// más realistas que árboles a mano y testean la cadena completa.
    #[test]
    fn python_function_indents_body() {
        use minga_core::parse::Dialect;
        let src = "def add(a, b):\n    return a + b\n";
        let node = Dialect::Python.parse(src).expect("parse");
        let out = render_source(&node);
        assert!(out.contains("def add"), "header presente: {out:?}");
        assert!(out.contains(":\n"), "header cerrado con `:`: {out:?}");
        assert!(out.contains("    return"), "cuerpo indentado: {out:?}");
    }

    #[test]
    fn python_if_else_keeps_branches_at_same_level() {
        use minga_core::parse::Dialect;
        let src = "if x:\n    a = 1\nelse:\n    a = 2\n";
        let node = Dialect::Python.parse(src).expect("parse");
        let out = render_source(&node);
        // ambos branches deben estar al nivel base (sin indent), y sus
        // cuerpos un nivel adentro.
        assert!(out.contains("\nelse:"), "else al nivel base: {out:?}");
        assert!(out.contains("    a = 1"), "rama if indentada: {out:?}");
        assert!(out.contains("    a = 2"), "rama else indentada: {out:?}");
    }

    #[test]
    fn python_class_with_method() {
        use minga_core::parse::Dialect;
        let src = "class C:\n    def m(self):\n        return self.x\n";
        let node = Dialect::Python.parse(src).expect("parse");
        let out = render_source(&node);
        assert!(out.contains("class C:"), "header de clase: {out:?}");
        assert!(out.contains("    def m"), "método indentado 4: {out:?}");
        assert!(out.contains("        return"), "cuerpo del método indentado 8: {out:?}");
    }
}
