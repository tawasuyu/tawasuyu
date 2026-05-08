//! Hash α-equivalente.
//!
//! Dos términos que difieren *solo* en los nombres de variables ligadas
//! producen el mismo hash. Los nombres de funciones, los identificadores
//! libres y los constructores (variantes, tipos) **sí** afectan al hash:
//! forman parte de la interfaz pública o discriminan el término.
//!
//! Implementación: durante el recorrido se mantiene una pila de scopes.
//! Al encontrar un binder reconocido, su nombre se empuja sobre la pila;
//! al salir del scope, se descarta. Las referencias a identificadores se
//! buscan desde la cima:
//! - si están, se emite un índice estilo de Bruijn (offset desde la cima);
//! - si no, se emite el nombre literal (variable libre).
//!
//! **Distinción binder vs. constructor:** dentro de un patrón, un
//! `identifier` puede ser binder (`x`, `mi_var`) o constructor / variante
//! (`None`, `Ok`, `MAX_VAL`). La gramática no los distingue; usamos la
//! convención de Rust: minúscula inicial (o `_` seguido de letra) = binder,
//! mayúscula inicial = constructor. Cuando el grammar marca explícitamente
//! `field_name = "pattern"` (parámetros, lets), forzamos binder.
//!
//! **Cobertura del MVP:**
//! - Parámetros de `function_item` y `closure_expression`.
//! - Bindings de `let_declaration` dentro de `block`, con desestructura.
//! - Variable de `for_expression`.
//! - Brazos de `match` (`match_arm` con guarda; cada arm es un scope
//!   independiente).
//! - Patrones: `tuple_pattern`, `tuple_struct_pattern`, `struct_pattern`,
//!   `field_pattern` (forma completa y shorthand), `captured_pattern`
//!   (`n @ pat`), `range_pattern`, `slice_pattern`, `ref_pattern`,
//!   `reference_pattern`, `mut_pattern`.
//!
//! **Pendiente:** `if let`, `while let`, `let-else`, let-chains, `or_pattern`
//! con bindings (Rust requiere mismas variables en cada rama).

use crate::ast::SemanticNode;
use crate::cas::ContentHash;
use blake3::Hasher;

const TAG_NO_LEAF: u8 = 0;
const TAG_LEAF: u8 = 1;
const TAG_BINDER: u8 = 2;
const TAG_REF_BOUND: u8 = 3;
const TAG_REF_FREE: u8 = 4;

pub fn hash_node_alpha(node: &SemanticNode) -> ContentHash {
    let mut h = Hasher::new();
    let mut scope: Vec<String> = Vec::new();
    feed(&mut h, node, &mut scope);
    ContentHash(*h.finalize().as_bytes())
}

fn feed(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, node);

    match node.kind.as_str() {
        "function_item" | "closure_expression" => feed_callable(h, node, scope),
        "block" => feed_block(h, node, scope),
        "for_expression" => feed_for(h, node, scope),
        "match_arm" => feed_match_arm(h, node, scope),
        "identifier" if node.field_name.as_deref() == Some("pattern") => emit_binder_body(h),
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

fn emit_identifier_ref(h: &mut Hasher, node: &SemanticNode, scope: &Vec<String>) {
    h.update(&[TAG_NO_LEAF]);
    if let Some(t) = &node.leaf_text {
        if let Ok(name) = std::str::from_utf8(t) {
            if let Some(i) = scope.iter().rposition(|n| n == name) {
                let de_bruijn = (scope.len() - 1 - i) as u64;
                h.update(&[TAG_REF_BOUND]);
                h.update(&de_bruijn.to_le_bytes());
            } else {
                h.update(&[TAG_REF_FREE]);
                h.update(&(t.len() as u64).to_le_bytes());
                h.update(t);
            }
        } else {
            h.update(&[TAG_REF_FREE]);
            h.update(&(t.len() as u64).to_le_bytes());
            h.update(t);
        }
    } else {
        h.update(&[TAG_REF_FREE]);
        h.update(&[0u8; 8]);
    }
    h.update(&[0u8; 8]);
}

fn emit_binder_body(h: &mut Hasher) {
    h.update(&[TAG_NO_LEAF]);
    h.update(&[TAG_BINDER]);
    h.update(&[0u8; 8]);
}

fn emit_binder_node(h: &mut Hasher, node: &SemanticNode) {
    write_kind_and_field(h, node);
    emit_binder_body(h);
}

fn emit_leaf_marker(h: &mut Hasher, node: &SemanticNode) {
    match &node.leaf_text {
        Some(t) => {
            h.update(&[TAG_LEAF]);
            h.update(&(t.len() as u64).to_le_bytes());
            h.update(t);
        }
        None => {
            h.update(&[TAG_NO_LEAF]);
        }
    }
}

fn feed_callable(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        if c.field_name.as_deref() == Some("parameters") {
            collect_callable_binders(c, &mut binders);
        }
    }

    let scope_before = scope.len();
    scope.extend(binders);

    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.field_name.as_deref() == Some("parameters") {
            feed_callable_params(h, c);
        } else {
            feed(h, c, scope);
        }
    }

    scope.truncate(scope_before);
}

fn feed_block(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let scope_before = scope.len();
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.kind == "let_declaration" {
            feed_let(h, c, scope);
            for cc in &c.children {
                if cc.field_name.as_deref() == Some("pattern") {
                    collect_pattern_binders(cc, scope);
                }
            }
        } else {
            feed(h, c, scope);
        }
    }
    scope.truncate(scope_before);
}

fn feed_let(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, node);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.field_name.as_deref() == Some("pattern") {
            feed_pattern(h, c);
        } else {
            feed(h, c, scope);
        }
    }
}

fn feed_for(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        if c.field_name.as_deref() == Some("pattern") {
            collect_pattern_binders(c, &mut binders);
        }
    }

    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        match c.field_name.as_deref() {
            Some("pattern") => feed_pattern(h, c),
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

fn feed_match_arm(h: &mut Hasher, node: &SemanticNode, scope: &mut Vec<String>) {
    h.update(&[TAG_NO_LEAF]);

    let mut binders: Vec<String> = Vec::new();
    for c in &node.children {
        if c.field_name.as_deref() == Some("pattern") {
            collect_match_pattern_binders(c, &mut binders);
        }
    }

    let scope_before = scope.len();
    scope.extend(binders);

    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.field_name.as_deref() == Some("pattern") {
            if c.kind == "match_pattern" {
                feed_match_pattern_split(h, c, scope);
            } else {
                feed_pattern(h, c);
            }
        } else {
            feed(h, c, scope);
        }
    }

    scope.truncate(scope_before);
}

fn feed_match_pattern_split(h: &mut Hasher, mp: &SemanticNode, scope: &mut Vec<String>) {
    write_kind_and_field(h, mp);
    emit_leaf_marker(h, mp);
    h.update(&(mp.children.len() as u64).to_le_bytes());
    for c in &mp.children {
        if c.field_name.as_deref() == Some("condition") {
            feed(h, c, scope);
        } else {
            feed_pattern(h, c);
        }
    }
}

fn collect_match_pattern_binders(p: &SemanticNode, out: &mut Vec<String>) {
    if p.kind == "match_pattern" {
        for c in &p.children {
            if c.field_name.as_deref() != Some("condition") {
                collect_pattern_binders(c, out);
            }
        }
    } else {
        collect_pattern_binders(p, out);
    }
}

fn feed_callable_params(h: &mut Hasher, params: &SemanticNode) {
    write_kind_and_field(h, params);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(params.children.len() as u64).to_le_bytes());
    for c in &params.children {
        match c.kind.as_str() {
            "parameter" => feed_parameter(h, c),
            _ => feed_pattern(h, c),
        }
    }
}

fn feed_parameter(h: &mut Hasher, node: &SemanticNode) {
    write_kind_and_field(h, node);
    h.update(&[TAG_NO_LEAF]);
    h.update(&(node.children.len() as u64).to_le_bytes());
    for c in &node.children {
        if c.field_name.as_deref() == Some("pattern") {
            feed_pattern(h, c);
        } else {
            feed_as_literal(h, c);
        }
    }
}

/// Pattern-aware emitter. Within a pattern, identifiers split into two
/// roles: binders (introduce a new local) and constructors (variant or
/// path references). The disambiguation rule mirrors Rust's: a `pattern`
/// field forces binder; otherwise lowercase initial = binder, uppercase =
/// constructor.
fn feed_pattern(h: &mut Hasher, node: &SemanticNode) {
    write_kind_and_field(h, node);
    match node.kind.as_str() {
        "identifier" => {
            if is_binder_identifier(node) {
                emit_binder_body(h);
            } else {
                emit_leaf_marker(h, node);
                h.update(&[0u8; 8]);
            }
        }
        "tuple_pattern" | "ref_pattern" | "reference_pattern" | "mut_pattern" | "slice_pattern" => {
            h.update(&[TAG_NO_LEAF]);
            h.update(&(node.children.len() as u64).to_le_bytes());
            for c in &node.children {
                feed_pattern(h, c);
            }
        }
        "tuple_struct_pattern" => {
            h.update(&[TAG_NO_LEAF]);
            h.update(&(node.children.len() as u64).to_le_bytes());
            for c in &node.children {
                if c.field_name.as_deref() == Some("type") {
                    feed_as_literal(h, c);
                } else {
                    feed_pattern(h, c);
                }
            }
        }
        "struct_pattern" => {
            h.update(&[TAG_NO_LEAF]);
            h.update(&(node.children.len() as u64).to_le_bytes());
            for c in &node.children {
                if c.field_name.as_deref() == Some("type") {
                    feed_as_literal(h, c);
                } else if c.kind == "field_pattern" {
                    feed_field_pattern(h, c);
                } else {
                    feed_as_literal(h, c);
                }
            }
        }
        "captured_pattern" => {
            h.update(&[TAG_NO_LEAF]);
            h.update(&(node.children.len() as u64).to_le_bytes());
            let mut named_binder = false;
            for c in &node.children {
                if !named_binder && c.kind == "identifier" {
                    emit_binder_node(h, c);
                    named_binder = true;
                } else {
                    feed_pattern(h, c);
                }
            }
        }
        _ => feed_as_literal(h, node),
    }
}

fn feed_field_pattern(h: &mut Hasher, fp: &SemanticNode) {
    write_kind_and_field(h, fp);
    let has_pattern = fp
        .children
        .iter()
        .any(|c| c.field_name.as_deref() == Some("pattern"));
    h.update(&[TAG_NO_LEAF]);
    h.update(&(fp.children.len() as u64).to_le_bytes());
    for c in &fp.children {
        if has_pattern {
            if c.field_name.as_deref() == Some("pattern") {
                feed_pattern(h, c);
            } else {
                feed_as_literal(h, c);
            }
        } else if matches!(
            c.kind.as_str(),
            "identifier" | "shorthand_field_identifier" | "field_identifier"
        ) {
            emit_binder_node(h, c);
        } else {
            feed_as_literal(h, c);
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

fn collect_callable_binders(params: &SemanticNode, out: &mut Vec<String>) {
    for c in &params.children {
        match c.kind.as_str() {
            "parameter" => {
                for cc in &c.children {
                    if cc.field_name.as_deref() == Some("pattern") {
                        collect_pattern_binders(cc, out);
                    }
                }
            }
            _ => collect_pattern_binders(c, out),
        }
    }
}

fn collect_pattern_binders(p: &SemanticNode, out: &mut Vec<String>) {
    match p.kind.as_str() {
        "identifier" => {
            if is_binder_identifier(p) {
                push_identifier_name(p, out);
            }
        }
        "tuple_pattern" | "ref_pattern" | "reference_pattern" | "mut_pattern" | "slice_pattern" => {
            for c in &p.children {
                collect_pattern_binders(c, out);
            }
        }
        "tuple_struct_pattern" => {
            for c in &p.children {
                if c.field_name.as_deref() != Some("type") {
                    collect_pattern_binders(c, out);
                }
            }
        }
        "struct_pattern" => {
            for c in &p.children {
                if c.kind == "field_pattern" {
                    collect_field_pattern_binders(c, out);
                }
            }
        }
        "captured_pattern" => {
            let mut named_binder = false;
            for c in &p.children {
                if !named_binder && c.kind == "identifier" {
                    push_identifier_name(c, out);
                    named_binder = true;
                } else {
                    collect_pattern_binders(c, out);
                }
            }
        }
        _ => {}
    }
}

fn collect_field_pattern_binders(fp: &SemanticNode, out: &mut Vec<String>) {
    let has_pattern = fp
        .children
        .iter()
        .any(|c| c.field_name.as_deref() == Some("pattern"));
    if has_pattern {
        for c in &fp.children {
            if c.field_name.as_deref() == Some("pattern") {
                collect_pattern_binders(c, out);
            }
        }
    } else {
        for c in &fp.children {
            if matches!(
                c.kind.as_str(),
                "identifier" | "shorthand_field_identifier" | "field_identifier"
            ) {
                push_identifier_name(c, out);
            }
        }
    }
}

fn push_identifier_name(node: &SemanticNode, out: &mut Vec<String>) {
    if let Some(t) = &node.leaf_text {
        if let Ok(s) = std::str::from_utf8(t) {
            out.push(s.to_string());
        }
    }
}

/// Determina si un `identifier` en posición de patrón se interpreta como
/// binder. Reglas:
/// - Si tiene `field_name == "pattern"` (parámetros, lets), siempre es binder.
/// - Si su nombre comienza con minúscula, es binder.
/// - Si comienza con `_` seguido de letra/dígito, es binder (convención
///   Rust para "intencionalmente sin usar").
/// - Resto: constructor / variante / constante (literal).
fn is_binder_identifier(node: &SemanticNode) -> bool {
    if node.field_name.as_deref() == Some("pattern") {
        return true;
    }
    let Some(t) = &node.leaf_text else { return false };
    let Ok(s) = std::str::from_utf8(t) else { return false };
    is_binder_name(s)
}

fn is_binder_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some('_') => chars
            .next()
            .map_or(false, |c| c.is_lowercase() || c.is_ascii_digit() || c == '_'),
        Some(c) => c.is_lowercase(),
        None => false,
    }
}

fn write_kind_and_field(h: &mut Hasher, node: &SemanticNode) {
    write_str(h, &node.kind);
    match &node.field_name {
        Some(f) => {
            h.update(&[1]);
            write_str(h, f);
        }
        None => {
            h.update(&[0]);
        }
    }
}

fn write_str(h: &mut Hasher, s: &str) {
    h.update(&(s.len() as u64).to_le_bytes());
    h.update(s.as_bytes());
}
