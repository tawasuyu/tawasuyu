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
//!   estructura de llaves. Para lenguajes con bloques por llaves
//!   (Rust/TS/JS/Go) sale legible; Python —cuya estructura vive en la
//!   indentación, y la indentación es trivia— sale como una secuencia
//!   de tokens en pocas líneas. Es esperado: el hash es de la
//!   estructura, no del formato.
//!
//! - [`render_sexp`]: el árbol como S-expression indentada. Vista
//!   exacta y sin pérdida de lo que el store guarda de verdad.

use minga_core::SemanticNode;

/// Reconstruye el código fuente de un subárbol en forma canónica.
///
/// Recolecta los tokens hoja en orden y los re-imprime con un
/// pretty-printer mínimo, consciente de llaves: indenta tras `{`,
/// desindenta antes de `}`, corta línea tras `;`. El resultado termina
/// siempre con exactamente un `\n`.
pub fn render_source(node: &SemanticNode) -> String {
    let mut tokens = Vec::new();
    collect_leaves(node, &mut tokens);
    pretty_print(&tokens)
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
}
