//! α-hashing per-language para JavaScript / TypeScript.
//!
//! Las dos gramáticas comparten la mayoría de los kinds (TypeScript
//! es JS + type annotations), así que un solo profile las cubre. El
//! caller (`hash_alpha_with`) despacha tanto `Dialect::JavaScript`
//! como `Dialect::TypeScript` acá.
//!
//! Cobertura:
//! - **`function_declaration`**, **`function_expression`**,
//!   **`method_definition`**, **`generator_function_declaration`**:
//!   parameters introducen binders al body.
//! - **`arrow_function`**: parameters (formal_parameters O identifier
//!   directo si es shorthand `x => ...`) introducen binder(es) al body.
//! - **`statement_block`**: cualquier `lexical_declaration` (let/const)
//!   o `variable_declaration` (var) dentro del block introduce binders
//!   al resto del block.
//! - **`for_in_statement`** (cubre tanto `for (x in obj)` como
//!   `for (x of arr)` en tree-sitter-javascript): el `left` es
//!   binder al `body`.
//! - **`for_statement`**: el `initializer` (lexical_declaration)
//!   introduce binder(es) al `condition`, `increment` y `body`.
//! - **`catch_clause`**: el `parameter` introduce binder al `body`.
//!
//! TypeScript-specific: `type` annotations (`x: number`) viajan como
//! children con field=type que se feedean por el path normal — el
//! tipo afecta el hash (cambiar de `number` a `string` rompe
//! α-equivalencia, intencionalmente).
//!
//! Pendientes (scope acotado):
//! - Destructuring (`const {a, b} = obj`, `const [x, y] = arr`).
//! - Class fields y constructor con `this.x = ...`.
//! - Hoisting de `var` a function scope (hoy se trata como block-scoped).

use crate::alpha::common::{
    emit_binder_body, emit_identifier_ref, emit_leaf_marker, push_identifier_name,
    write_kind_and_field, TAG_NO_LEAF,
};
use crate::ast::SemanticNode;
use crate::cas::ContentHash;
use blake3::Hasher;

pub fn hash_node_alpha_ecmascript(node: &SemanticNode) -> ContentHash {
    let mut h = Hasher::new();
    let mut scope: Vec<String> = Vec::new();
    feed(&mut h, node, &mut scope);
    ContentHash(*h.finalize().as_bytes())
}

fn feed(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, node);
    match node.kind.as_str() {
        "function_declaration"
        | "function_expression"
        | "generator_function_declaration"
        | "generator_function"
        | "method_definition" => feed_callable(h, node, scope),
        "arrow_function" => feed_arrow(h, node, scope),
        "statement_block" => feed_block(h, node, scope),
        "for_in_statement" => feed_for_in(h, node, scope),
        "for_statement" => feed_for(h, node, scope),
        "catch_clause" => feed_catch(h, node, scope),
        // Lexical declarations dispatcheadas también desde feed
        // general, no sólo desde feed_block. Necesario para
        // for_statement (initializer) y otros contextos donde una
        // declaration aparece sin ser hijo directo de un block.
        "lexical_declaration" | "variable_declaration" => feed_var_decl(h, node, scope),
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

/// Callable estándar: parameters → body.
fn feed_callable(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        if c.field_name.as_deref() == Some("parameters") {
            collect_formal_param_binders(c, &mut binders);
        }
    }

    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        match c.field_name.as_deref() {
            Some("parameters") => feed_formal_params(h, c, scope),
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

/// Arrow function: dos formas. `x => body` (single identifier) o
/// `(x, y) => body` (formal_parameters). Detectamos cuál.
fn feed_arrow(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        match c.field_name.as_deref() {
            Some("parameter") => {
                // `x => ...` — el identifier solo.
                if c.kind == "identifier" {
                    push_identifier_name(c, &mut binders);
                }
            }
            Some("parameters") => {
                collect_formal_param_binders(c, &mut binders);
            }
            _ => {}
        }
    }

    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        match c.field_name.as_deref() {
            Some("parameter") => emit_arrow_single_binder(h, c),
            Some("parameters") => feed_formal_params(h, c, scope),
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

fn emit_arrow_single_binder(h: &mut Hasher, node: &SemanticNode) {
    write_kind_and_field(h, node);
    if node.kind == "identifier" {
        emit_binder_body(h);
    } else {
        // Otra forma (rare); fallback al feed normal sin binder.
        emit_leaf_marker(h, node);
        h.update(&(node.children.len() as u64).to_le_bytes());
    }
}

/// Statement block: `let`/`const`/`var` declarations introducen
/// binders al resto del block (lexical scope).
fn feed_block(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let scope_before = scope.len();
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        match c.kind.as_str() {
            "lexical_declaration" | "variable_declaration" => {
                feed_var_decl(h, c, scope);
                collect_var_decl_binders(c, scope);
            }
            _ => feed(h, c, scope),
        }
    }
    scope.truncate(scope_before);
}

/// Procesa una let/const/var declaration: el `value` se evalúa en el
/// scope previo (los binders aún no existen para sí mismos); el
/// `name` se emite como binder anónimo.
fn feed_var_decl(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, node);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.kind == "variable_declarator" {
            feed_declarator(h, c, scope);
        } else {
            feed(h, c, scope);
        }
    }
}

fn feed_declarator(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, node);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        match c.field_name.as_deref() {
            Some("name") if c.kind == "identifier" => emit_named_binder(h, c),
            _ => feed(h, c, scope),
        }
    }
}

fn collect_var_decl_binders(node: &SemanticNode, out: &mut Vec<String>) {
    for c in &node.children {
        if c.kind == "variable_declarator" {
            for cc in &c.children {
                if cc.field_name.as_deref() == Some("name") && cc.kind == "identifier" {
                    push_identifier_name(cc, out);
                }
            }
        }
    }
}

/// `for (x of arr)` o `for (x in obj)`. left = identifier (con
/// posible kind=const/let prefix para lexical decl), right = expr,
/// body = block.
fn feed_for_in(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        if c.field_name.as_deref() == Some("left") && c.kind == "identifier" {
            push_identifier_name(c, &mut binders);
        }
    }

    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        match c.field_name.as_deref() {
            Some("left") if c.kind == "identifier" => emit_named_binder(h, c),
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

/// `for (let i = 0; i < n; i++) { body }`. El initializer (lexical
/// decl) introduce binders que viven en condition + increment + body.
fn feed_for(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        if c.field_name.as_deref() == Some("initializer")
            && (c.kind == "lexical_declaration" || c.kind == "variable_declaration")
        {
            collect_var_decl_binders(c, &mut binders);
        }
    }

    let scope_before = scope.len();
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        match c.field_name.as_deref() {
            Some("initializer") => {
                feed(h, c, scope);
                // Tras procesar el initializer extendemos scope para
                // que condition/increment/body lo vean.
                scope.extend(binders.iter().cloned());
            }
            _ => feed(h, c, scope),
        }
    }
    scope.truncate(scope_before);
}

/// `catch (e) { body }`. parameter es identifier → binder al body.
fn feed_catch(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        if c.field_name.as_deref() == Some("parameter") && c.kind == "identifier" {
            push_identifier_name(c, &mut binders);
        }
    }

    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        match c.field_name.as_deref() {
            Some("parameter") if c.kind == "identifier" => emit_named_binder(h, c),
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

/// formal_parameters de function declarations. Soporta:
/// - `identifier` (param simple).
/// - `required_parameter` (TypeScript: `x: number`).
/// - `optional_parameter` (TypeScript: `x?: number`).
/// - `rest_pattern` / `rest_parameter` (`...rest`).
fn feed_formal_params(h: &mut Hasher, params: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, params);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(params.children.len() as u64).to_le_bytes());
    for c in &params.children {
        match c.kind.as_str() {
            "identifier" => emit_named_binder(h, c),
            "required_parameter" | "optional_parameter" => {
                feed_typed_param(h, c, scope);
            }
            "rest_pattern" | "rest_parameter" => {
                feed_rest_param(h, c, scope);
            }
            _ => feed(h, c, scope),
        }
    }
}

fn feed_typed_param(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, node);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(node.children.len() as u64).to_le_bytes());
    let mut named_binder = false;
    for c in &node.children {
        if !named_binder && c.kind == "identifier" {
            emit_named_binder(h, c);
            named_binder = true;
        } else {
            feed(h, c, scope);
        }
    }
}

fn feed_rest_param(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, node);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.kind == "identifier" {
            emit_named_binder(h, c);
        } else {
            feed(h, c, scope);
        }
    }
}

fn collect_formal_param_binders(params: &SemanticNode, out: &mut Vec<String>) {
    for c in &params.children {
        match c.kind.as_str() {
            "identifier" => push_identifier_name(c, out),
            "required_parameter" | "optional_parameter" | "rest_pattern" | "rest_parameter" => {
                if let Some(ident) = c.children.iter().find(|cc| cc.kind == "identifier") {
                    push_identifier_name(ident, out);
                }
            }
            _ => {}
        }
    }
}

fn emit_named_binder(h: &mut Hasher, node: &SemanticNode) {
    write_kind_and_field(h, node);
    emit_binder_body(h);
}
