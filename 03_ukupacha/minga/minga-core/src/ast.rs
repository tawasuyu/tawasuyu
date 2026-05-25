use tree_sitter::Node;

/// Nodo de AST normalizado: descarta posiciones, whitespace y trivia
/// (comentarios marcados como `extra` en la gramática). Dos fragmentos de
/// código semánticamente equivalentes producen árboles idénticos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticNode {
    pub kind: String,
    pub field_name: Option<String>,
    pub leaf_text: Option<Vec<u8>>,
    pub children: Vec<SemanticNode>,
}

impl SemanticNode {
    pub fn from_tree_sitter(node: Node<'_>, source: &[u8]) -> Self {
        Self::build(node, source, None)
    }

    fn build(node: Node<'_>, source: &[u8], field_name: Option<String>) -> Self {
        let kind = node.kind().to_string();
        let mut children = Vec::new();

        // Incluimos todos los hijos no-`extra`: nombrados (rules de la
        // gramática) y anónimos (tokens literales como operadores y
        // separadores). Lo único que descartamos son `extras` —
        // comentarios y whitespace en gramáticas tree-sitter — que es
        // exactamente la invariancia que queremos: dos formas con el
        // mismo contenido y estructura producen el mismo árbol.
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if !child.is_extra() {
                    let field = cursor.field_name().map(|s| s.to_string());
                    children.push(Self::build(child, source, field));
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }

        let leaf_text = if children.is_empty() {
            let range = node.byte_range();
            Some(source[range].to_vec())
        } else {
            None
        };

        SemanticNode { kind, field_name, leaf_text, children }
    }
}
