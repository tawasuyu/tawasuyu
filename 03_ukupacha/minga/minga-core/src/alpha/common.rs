//! Primitives compartidos entre todos los profiles α-hashing.
//!
//! Cada profile per-language (rust, python, ecmascript, go) tiene su
//! propia lógica de "qué nodos introducen binders" y "cómo distinguir
//! binders de constructors". Pero el format del wire del hash
//! (TAG_LEAF, TAG_BINDER, índice de Bruijn) es universal: lo emitimos
//! desde acá para garantizar que dos lenguajes con la misma
//! estructura semántica produzcan hashes comparables a nivel de bits.

use crate::ast::SemanticNode;
use blake3::Hasher;

pub const TAG_NO_LEAF: u8 = 0;
pub const TAG_LEAF: u8 = 1;
pub const TAG_BINDER: u8 = 2;
pub const TAG_REF_BOUND: u8 = 3;
pub const TAG_REF_FREE: u8 = 4;

/// Emite el kind del nodo + presencia/ausencia de field_name.
pub fn write_kind_and_field(h: &mut Hasher, node: &SemanticNode) {
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

pub fn write_str(h: &mut Hasher, s: &str) {
    h.update(&(s.len() as u64).to_le_bytes());
    h.update(s.as_bytes());
}

/// Emite el marker de leaf: TAG_LEAF + bytes del leaf si lo hay,
/// TAG_NO_LEAF si no.
pub fn emit_leaf_marker(h: &mut Hasher, node: &SemanticNode) {
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

/// Emite un binder anónimo: el contenido textual NO afecta el hash.
/// Esta es la primitiva de α-equivalencia: dos términos que sólo
/// difieren en nombres de variables ligadas hashean idénticos.
pub fn emit_binder_body(h: &mut Hasher) {
    h.update(&[TAG_NO_LEAF]);
    h.update(&[TAG_BINDER]);
    h.update(&[0u8; 8]);
}

/// Emite el kind del nodo + binder body. Atajo para nodos cuyo único
/// rol es ser binder (e.g. un identifier en posición de pattern).
pub fn emit_binder_node(h: &mut Hasher, node: &SemanticNode) {
    write_kind_and_field(h, node);
    emit_binder_body(h);
}

/// Emite un identifier referencia: si está en scope, índice de
/// Bruijn (offset desde la cima); si no, nombre literal (variable
/// libre).
pub fn emit_identifier_ref(h: &mut Hasher, node: &SemanticNode, scope: &[String]) {
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

/// Push el nombre del identifier al vector de binders, si tiene
/// leaf_text válido. Helper común para todos los `collect_binders`.
pub fn push_identifier_name(node: &SemanticNode, out: &mut Vec<String>) {
    if let Some(t) = &node.leaf_text {
        if let Ok(s) = std::str::from_utf8(t) {
            out.push(s.to_string());
        }
    }
}
