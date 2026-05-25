//! α-hashing per-language para Go.
//!
//! Cobertura:
//! - **`function_declaration`**, **`method_declaration`**,
//!   **`func_literal`** (closure): `parameter_list` introduce
//!   binder(es) al `body`.
//! - **`parameter_declaration`**: puede agrupar varios names con un
//!   tipo (`a, b int`). Cada `name` es binder; `type` viaja como
//!   referencia.
//! - **`block`**: `short_var_declaration` (`x := ...`) introduce
//!   binders al resto del block.
//! - **`for_statement`** con **`range_clause`** (`for k, v := range m`):
//!   los identifiers del `left` son binders al `body`.
//! - **`for_statement`** con **`for_clause`** (C-style `for i := 0; i < n; i++`):
//!   el `initializer` (short_var_declaration) introduce binders al
//!   condition + update + body.
//! - **`if_statement`** con **`initializer`**: binders del
//!   short_var_declaration viven en condition + consequence + alternative.
//!
//! Pendientes (scope acotado):
//! - `var_declaration` (`var x = ...`) tratado como literal por
//!   ahora; introduce binder al scope envolvente igual que
//!   short_var_declaration pero distinto kind.
//! - `type_switch_statement` con assertion binding.
//! - `select` statements con send/receive binding.

use crate::alpha::common::{
    emit_binder_body, emit_identifier_ref, emit_leaf_marker, push_identifier_name,
    write_kind_and_field, TAG_NO_LEAF,
};
use crate::ast::SemanticNode;
use crate::cas::ContentHash;
use blake3::Hasher;

pub fn hash_node_alpha_go(node: &SemanticNode) -> ContentHash {
    let mut h = Hasher::new();
    let mut scope: Vec<String> = Vec::new();
    feed(&mut h, node, &mut scope);
    ContentHash(*h.finalize().as_bytes())
}

fn feed(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, node);
    match node.kind.as_str() {
        "function_declaration" | "method_declaration" | "func_literal" => {
            feed_callable(h, node, scope)
        }
        "block" => feed_block(h, node, scope),
        "for_statement" => feed_for_statement(h, node, scope),
        "if_statement" => feed_if_statement(h, node, scope),
        // Dispatcheados también fuera de block/for/if para que sus
        // identifiers se emitan como binders cuando aparecen en
        // contextos como range_clause o initializer de if/for.
        "short_var_declaration" => feed_short_var_decl(h, node, scope),
        "range_clause" => feed_range_clause(h, node, scope),
        "identifier" => emit_identifier_ref(h, node, scope),
        _ => feed_default(h, node, scope),
    }
}

/// `for k, v := range m` — el `left` (expression_list) tiene
/// identifiers que son binders. El `right` se evalúa como referencia
/// normal (es la fuente de iteración).
fn feed_range_clause(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.field_name.as_deref() == Some("left") {
            feed_short_var_left(h, c);
        } else {
            feed(h, c, scope);
        }
    }
}

fn feed_default(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    emit_leaf_marker(h, node);
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        feed(h, c, scope);
    }
}

fn feed_callable(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        if c.field_name.as_deref() == Some("parameters") {
            collect_parameter_list_binders(c, &mut binders);
        }
    }

    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        match c.field_name.as_deref() {
            Some("parameters") => feed_parameter_list(h, c, scope),
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

fn feed_parameter_list(h: &mut Hasher, params: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, params);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(params.children.len() as u64).to_le_bytes());
    for c in &params.children {
        if c.kind == "parameter_declaration" {
            feed_parameter_declaration(h, c, scope);
        } else {
            feed(h, c, scope);
        }
    }
}

/// `a, b int` — todos los `name=identifier` son binders; `type`
/// viaja como referencia normal (puede mencionar tipos importados).
fn feed_parameter_declaration(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, node);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.field_name.as_deref() == Some("name") && c.kind == "identifier" {
            emit_named_binder(h, c);
        } else {
            feed(h, c, scope);
        }
    }
}

fn collect_parameter_list_binders(params: &SemanticNode, out: &mut Vec<String>) {
    for c in &params.children {
        if c.kind == "parameter_declaration" {
            for cc in &c.children {
                if cc.field_name.as_deref() == Some("name") && cc.kind == "identifier" {
                    push_identifier_name(cc, out);
                }
            }
        }
    }
}

/// Block: `short_var_declaration` introduce binders al resto.
fn feed_block(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let scope_before = scope.len();
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.kind == "short_var_declaration" {
            feed_short_var_decl(h, c, scope);
            collect_short_var_binders(c, scope);
        } else {
            feed(h, c, scope);
        }
    }
    scope.truncate(scope_before);
}

fn feed_short_var_decl(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, node);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.field_name.as_deref() == Some("left") {
            feed_short_var_left(h, c);
        } else {
            feed(h, c, scope);
        }
    }
}

fn feed_short_var_left(h: &mut Hasher, node: &SemanticNode) {
    write_kind_and_field(h, node);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.kind == "identifier" {
            emit_named_binder(h, c);
        } else {
            // separadores ',' y otros tokens — emit literal.
            emit_leaf_marker(h, c);
            h.update(&(c.children.len() as u64).to_le_bytes());
        }
    }
}

fn collect_short_var_binders(node: &SemanticNode, out: &mut Vec<String>) {
    for c in &node.children {
        if c.field_name.as_deref() == Some("left") {
            for cc in &c.children {
                if cc.kind == "identifier" {
                    push_identifier_name(cc, out);
                }
            }
        }
    }
}

/// `for k, v := range m { body }` o `for i := 0; i < n; i++ { body }`.
fn feed_for_statement(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        match c.kind.as_str() {
            "range_clause" => {
                for cc in &c.children {
                    if cc.field_name.as_deref() == Some("left") {
                        for ccc in &cc.children {
                            if ccc.kind == "identifier" {
                                push_identifier_name(ccc, &mut binders);
                            }
                        }
                    }
                }
            }
            "for_clause" => {
                for cc in &c.children {
                    if cc.field_name.as_deref() == Some("initializer")
                        && cc.kind == "short_var_declaration"
                    {
                        collect_short_var_binders(cc, &mut binders);
                    }
                }
            }
            _ => {}
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

/// `if x := init(); cond { ... } else { ... }`. El initializer
/// introduce binders que viven en condition + consequence +
/// alternative.
fn feed_if_statement(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        if c.field_name.as_deref() == Some("initializer")
            && c.kind == "short_var_declaration"
        {
            collect_short_var_binders(c, &mut binders);
        }
    }

    let scope_before = scope.len();
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        match c.field_name.as_deref() {
            Some("initializer") => {
                feed(h, c, scope);
                scope.extend(binders.iter().cloned());
            }
            _ => feed(h, c, scope),
        }
    }
    scope.truncate(scope_before);
}

fn emit_named_binder(h: &mut Hasher, node: &SemanticNode) {
    write_kind_and_field(h, node);
    emit_binder_body(h);
}
