//! α-hashing per-language para Python.
//!
//! Cobertura:
//! - **`function_definition`** y **`lambda`**: parámetros introducen
//!   binders al body. Soporta defaults (`def f(x=1)`) y type hints
//!   (`def f(x: int)`) — el binder es el identifier; el default y el
//!   type viajan como expresiones referenciables al scope previo.
//! - **`for_statement`**: el `left` (identifier o tuple_pattern)
//!   introduce binder(es) al `body`.
//! - **Comprehensions**: `list_comprehension`, `set_comprehension`,
//!   `dictionary_comprehension`, `generator_expression`. Cada
//!   `for_in_clause` introduce binder(es) que viven en el `body` +
//!   `if_clause`s + `for_in_clause`s siguientes (semántica de scope
//!   incremental de Python).
//! - **`with_statement`**: `with X() as y:` introduce `y` al body.
//!
//! Python NO distingue binders por capitalización (a diferencia de
//! Rust con `Some` vs `x`). En posición de parámetro/for-target,
//! todo identifier es binder.
//!
//! Pendientes (no cubiertos hoy, scope acotado):
//! - `class_definition` y métodos (`self` no es binder explícito en
//!   la firma; el primer parámetro recibe nombre arbitrario).
//! - `assignment` como introductor de scope (Python no tiene `let`
//!   explícito; un `x = 1` agrega x al scope global o local del
//!   bloque envolvente — manejarlo bien requiere análisis de scope
//!   que va más allá del α-hashing tradicional).
//! - Nested defaults, walrus operator (`:=`), starred patterns.

use crate::alpha::common::{
    emit_binder_body, emit_identifier_ref, emit_leaf_marker, push_identifier_name,
    write_kind_and_field, TAG_NO_LEAF,
};
use crate::ast::SemanticNode;
use crate::cas::ContentHash;
use blake3::Hasher;

pub fn hash_node_alpha_python(node: &SemanticNode) -> ContentHash {
    let mut h = Hasher::new();
    let mut scope: Vec<String> = Vec::new();
    feed(&mut h, node, &mut scope);
    ContentHash(*h.finalize().as_bytes())
}

fn feed(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, node);
    match node.kind.as_str() {
        "function_definition" => feed_function_definition(h, node, scope),
        "lambda" => feed_lambda(h, node, scope),
        "for_statement" => feed_for_statement(h, node, scope),
        "list_comprehension"
        | "set_comprehension"
        | "dictionary_comprehension"
        | "generator_expression" => feed_comprehension(h, node, scope),
        "with_statement" => feed_with_statement(h, node, scope),
        // Cuando un as_pattern_target aparece (típicamente dentro de
        // un with_clause), sus identifiers son binders. El scope ya
        // se extendió en feed_with_statement antes de llegar al body;
        // pero el target mismo necesita emitir binders anónimos para
        // que el hash no varíe con el nombre.
        "as_pattern_target" => feed_target_as_binders(h, node),
        "identifier" => emit_identifier_ref(h, node, scope),
        _ => feed_default(h, node, scope),
    }
}

fn feed_default(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    emit_leaf_marker(h, node);
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        feed(h, c, scope);
    }
}

/// `def f(x, y=1, z: int): body` → params son binders al body.
/// El `name` (identifier de la función) se trata como literal — no
/// es un binder local (es publicado al scope envolvente, no manejado
/// acá).
fn feed_function_definition(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        if c.field_name.as_deref() == Some("parameters") {
            collect_param_binders(c, &mut binders);
        }
    }

    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        match c.field_name.as_deref() {
            Some("parameters") => feed_params(h, c, scope),
            Some("body") => {
                let scope_before = scope.len();
                scope.extend(binders.iter().cloned());
                feed(h, c, scope);
                scope.truncate(scope_before);
            }
            Some("name") => {
                // Nombre de la función: viaja como literal (afecta el
                // hash, no es α-anónimo). Mismo tratamiento que en
                // Rust con `function_item.name`.
                feed_as_literal(h, c);
            }
            _ => feed(h, c, scope),
        }
    }
}

/// `lambda x, y: body` — params binders al body.
fn feed_lambda(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        if c.field_name.as_deref() == Some("parameters") {
            collect_param_binders(c, &mut binders);
        }
    }

    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        match c.field_name.as_deref() {
            Some("parameters") => feed_params(h, c, scope),
            Some("body") => {
                let scope_before = scope.len();
                scope.extend(binders.iter().cloned());
                feed(h, c, scope);
                scope.truncate(scope_before);
            }
            _ => feed(h, c, scope),
        }
    }
}

/// `for x in iterable: body` — x es binder al body.
fn feed_for_statement(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        if c.field_name.as_deref() == Some("left") {
            collect_target_binders(c, &mut binders);
        }
    }

    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        match c.field_name.as_deref() {
            Some("left") => feed_target_as_binders(h, c),
            Some("body") => {
                let scope_before = scope.len();
                scope.extend(binders.iter().cloned());
                feed(h, c, scope);
                scope.truncate(scope_before);
            }
            _ => feed(h, c, scope),
        }
    }
}

/// `[expr for x in xs if cond]` — los `for_in_clause` y `if_clause`
/// se procesan en orden: cada `for_in_clause` añade binders que
/// viven en lo siguiente. El `body` (la expresión final) ve TODOS
/// los binders acumulados.
fn feed_comprehension(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    // Recolectamos TODOS los binders de TODAS las for_in_clauses.
    // Python evalúa la comprehension de izquierda a derecha pero el
    // body ve todo; α-hashing colapsa eso a "todos visibles en body".
    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        if c.kind == "for_in_clause" {
            for cc in &c.children {
                if cc.field_name.as_deref() == Some("left") {
                    collect_target_binders(cc, &mut binders);
                }
            }
        }
    }

    let scope_before = scope.len();
    scope.extend(binders.iter().cloned());

    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.kind == "for_in_clause" {
            feed_for_in_clause(h, c, scope);
        } else {
            feed(h, c, scope);
        }
    }

    scope.truncate(scope_before);
}

/// `for x in xs` dentro de una comprehension. El `left` es binder
/// (anónimo); el `right` se evalúa en el scope previo (sin x).
/// Pero como `feed_comprehension` ya extendió el scope antes de
/// llamarnos, x sí está en scope para el right de un `for X in expr`
/// posterior — semántica correcta de comprehensions de Python.
fn feed_for_in_clause(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, node);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.field_name.as_deref() == Some("left") {
            feed_target_as_binders(h, c);
        } else {
            feed(h, c, scope);
        }
    }
}

/// `with X() as y, Z() as w: body` — los `as` introducen binders al body.
fn feed_with_statement(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        if c.kind == "with_clause" {
            collect_with_clause_binders(c, &mut binders);
        }
    }

    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        match c.field_name.as_deref() {
            Some("body") => {
                let scope_before = scope.len();
                scope.extend(binders.iter().cloned());
                feed(h, c, scope);
                scope.truncate(scope_before);
            }
            _ => feed(h, c, scope),
        }
    }
}

fn collect_with_clause_binders(node: &SemanticNode, out: &mut Vec<String>) {
    // En tree-sitter-python, with_item.value puede ser un as_pattern
    // que tiene su propio alias. Recursamos para encontrar cualquier
    // as_pattern_target en el subárbol.
    for c in &node.children {
        if c.kind == "with_item" {
            collect_as_pattern_targets(c, out);
        }
    }
}

fn collect_as_pattern_targets(node: &SemanticNode, out: &mut Vec<String>) {
    if node.kind == "as_pattern_target" {
        collect_target_binders(node, out);
        return;
    }
    for c in &node.children {
        collect_as_pattern_targets(c, out);
    }
}

/// Los parameters de def/lambda se procesan emitiendo cada
/// identifier como binder anónimo. Defaults / type hints / *args /
/// **kwargs se preservan literalmente (afectan el hash).
fn feed_params(h: &mut Hasher, params: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, params);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(params.children.len() as u64).to_le_bytes());
    for c in &params.children {
        match c.kind.as_str() {
            "identifier" => emit_param_binder(h, c),
            "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
                feed_complex_param(h, c, scope);
            }
            "list_splat_pattern" | "dictionary_splat_pattern" => {
                // *args, **kwargs: el binder es el identifier interno.
                feed_splat_param(h, c);
            }
            _ => feed(h, c, scope),
        }
    }
}

fn emit_param_binder(h: &mut Hasher, ident: &SemanticNode) {
    write_kind_and_field(h, ident);
    emit_binder_body(h);
}

/// `x: int`, `x = 1`, `x: int = 1` — el primer identifier es binder;
/// el resto (type, default) son referenciables.
fn feed_complex_param(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, node);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(node.children.len() as u64).to_le_bytes());
    let mut named_binder = false;
    for c in &node.children {
        if !named_binder && c.kind == "identifier" {
            emit_param_binder(h, c);
            named_binder = true;
        } else {
            feed(h, c, scope);
        }
    }
}

fn feed_splat_param(h: &mut Hasher, node: &SemanticNode) {
    write_kind_and_field(h, node);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.kind == "identifier" {
            emit_param_binder(h, c);
        } else {
            feed_as_literal(h, c);
        }
    }
}

fn collect_param_binders(params: &SemanticNode, out: &mut Vec<String>) {
    for c in &params.children {
        match c.kind.as_str() {
            "identifier" => push_identifier_name(c, out),
            "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
                if let Some(ident) = c.children.iter().find(|cc| cc.kind == "identifier") {
                    push_identifier_name(ident, out);
                }
            }
            "list_splat_pattern" | "dictionary_splat_pattern" => {
                if let Some(ident) = c.children.iter().find(|cc| cc.kind == "identifier") {
                    push_identifier_name(ident, out);
                }
            }
            _ => {}
        }
    }
}

/// El `left` de `for x in xs:` o de `with X as y:` puede ser un
/// identifier solo o una tupla destructurada (`for k, v in ...`).
fn collect_target_binders(target: &SemanticNode, out: &mut Vec<String>) {
    match target.kind.as_str() {
        "identifier" => push_identifier_name(target, out),
        "tuple_pattern" | "pattern_list" | "list_pattern" => {
            for c in &target.children {
                collect_target_binders(c, out);
            }
        }
        _ => {
            // Recursamos por si hay subnodos relevantes (e.g. parens).
            for c in &target.children {
                collect_target_binders(c, out);
            }
        }
    }
}

/// Emit del target como binders anónimos. Mismo recorrido que collect.
fn feed_target_as_binders(h: &mut Hasher, target: &SemanticNode) {
    write_kind_and_field(h, target);
    match target.kind.as_str() {
        "identifier" => emit_binder_body(h),
        "tuple_pattern" | "pattern_list" | "list_pattern" => {
            h.update(&[TAG_NO_LEAF]);
            h.update(&(target.children.len() as u64).to_le_bytes());
            for c in &target.children {
                feed_target_as_binders(h, c);
            }
        }
        _ => {
            // Fallback: literal (preserva la estructura textual).
            emit_leaf_marker(h, target);
            h.update(&(target.children.len() as u64).to_le_bytes());
            for c in &target.children {
                feed_target_as_binders(h, c);
            }
        }
    }
}

fn feed_as_literal(h: &mut Hasher, node: &SemanticNode) {
    write_kind_and_field(h, node);
    emit_leaf_marker(h, node);
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        feed_as_literal(h, c);
    }
}
