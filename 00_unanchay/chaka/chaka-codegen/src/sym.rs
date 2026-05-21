//! Tabla de símbolos del código generado: los campos del `struct
//! Program` y los nombres de condición, derivados del modelo de datos
//! resuelto que entrega `charka-ir`.

use std::collections::HashMap;

use charka_ir::{ConditionName, DataModel};

/// El tipo de campo lo aporta `charka-ir`; se reexporta para que el
/// resto del crate lo nombre como `crate::sym::FieldKind`.
pub(crate) use charka_ir::FieldKind;

/// Un campo del struct `Program` generado.
pub(crate) struct Field {
    /// Nombre COBOL en mayúsculas.
    pub cobol: String,
    /// Identificador Rust saneado y único.
    pub ident: String,
    /// Numérico o alfanumérico.
    pub kind: FieldKind,
    /// Valor inicial normalizado (de la cláusula `VALUE`).
    pub init: String,
    /// Si es una tabla (`OCCURS n`), su número de elementos.
    pub occurs: Option<u32>,
}

/// Los campos del programa y sus nombres de condición, indexados.
pub(crate) struct Symbols {
    pub fields: Vec<Field>,
    by_name: HashMap<String, usize>,
    conditions: HashMap<String, ConditionName>,
}

impl Symbols {
    /// Construye la tabla desde el modelo de datos resuelto.
    pub(crate) fn build(model: &DataModel) -> Self {
        let mut fields: Vec<Field> = model
            .fields
            .iter()
            .map(|f| Field {
                cobol: f.name.clone(),
                ident: sanitize_ident(&f.name),
                kind: f.kind,
                init: f.init.clone(),
                occurs: f.occurs,
            })
            .collect();
        dedup_idents(&mut fields);
        let by_name = fields
            .iter()
            .enumerate()
            .map(|(i, f)| (f.cobol.clone(), i))
            .collect();
        let conditions = model
            .conditions
            .iter()
            .map(|c| (c.name.clone(), c.clone()))
            .collect();
        Self {
            fields,
            by_name,
            conditions,
        }
    }

    /// Busca un campo por su nombre COBOL (sin distinguir mayúsculas).
    pub(crate) fn lookup(&self, cobol: &str) -> Option<&Field> {
        self.by_name
            .get(&cobol.to_uppercase())
            .map(|&i| &self.fields[i])
    }

    /// Busca un nombre de condición (un dato de nivel 88).
    pub(crate) fn condition(&self, name: &str) -> Option<&ConditionName> {
        self.conditions.get(&name.to_uppercase())
    }
}

/// Dos datos pueden compartir nombre (COBOL los califica); aquí, como
/// son campos de un struct, sus identificadores deben ser únicos.
fn dedup_idents(fields: &mut [Field]) {
    let mut seen: HashMap<String, u32> = HashMap::new();
    for f in fields.iter_mut() {
        let n = seen.entry(f.ident.clone()).or_insert(0);
        if *n > 0 {
            f.ident = format!("{}_{}", f.ident, n);
        }
        *n += 1;
    }
}

/// Convierte un nombre COBOL en un identificador Rust válido.
fn sanitize_ident(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c == '-' {
                '_'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        s = format!("f_{s}");
    }
    if is_rust_keyword(&s) {
        s.push('_');
    }
    s
}

/// El nombre del método de un párrafo. El párrafo implícito ("") es
/// `p_start`; el resto lleva el prefijo `p_`.
pub(crate) fn paragraph_method(name: &str) -> String {
    if name.is_empty() {
        return "p_start".to_string();
    }
    let body: String = name
        .chars()
        .map(|c| {
            if c == '-' {
                '_'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    format!("p_{body}")
}

/// ¿Es `s` una palabra reservada de Rust? (Para no chocar al nombrar
/// campos — un dato COBOL `MOVE` se volvería el keyword `move`.)
fn is_rust_keyword(s: &str) -> bool {
    matches!(
        s,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "box"
            | "yield"
    )
}
