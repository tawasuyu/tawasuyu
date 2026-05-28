//! Tabla de símbolos del código generado: los campos del `struct
//! Program` y los nombres de condición, derivados del modelo de datos
//! resuelto que entrega `chaka-ir`.

use std::collections::HashMap;

use chaka_ir::{ConditionName, Ir};

/// El tipo de campo lo aporta `chaka-ir`; se reexporta para que el
/// resto del crate lo nombre como `crate::sym::FieldKind`.
pub(crate) use chaka_ir::FieldKind;

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
    /// Si es un campo de edición, su PICTURE.
    pub edit: Option<String>,
}

/// Un fichero del programa generado.
pub(crate) struct FileSym {
    /// Nombre COBOL del fichero.
    pub cobol: String,
    /// Identificador Rust del campo `CobFile` (prefijo `file_`).
    pub ident: String,
    /// Ruta a la que está asignado.
    pub path: String,
    /// Nombre COBOL del registro asociado (su `FD`).
    pub record: String,
}

/// Los campos del programa, sus nombres de condición, sus grupos, sus
/// párrafos y sus ficheros.
pub(crate) struct Symbols {
    pub fields: Vec<Field>,
    by_name: HashMap<String, usize>,
    conditions: HashMap<String, ConditionName>,
    groups: HashMap<String, Vec<String>>,
    /// Los párrafos en orden: `(nombre COBOL, nombre de método Rust)`.
    pub paragraphs: Vec<(String, String)>,
    /// Los ficheros declarados.
    pub files: Vec<FileSym>,
}

impl Symbols {
    /// Construye la tabla desde el IR (su modelo de datos y párrafos).
    pub(crate) fn build(ir: &Ir) -> Self {
        let model = &ir.model;
        let mut fields: Vec<Field> = model
            .fields
            .iter()
            .map(|f| Field {
                cobol: f.name.clone(),
                ident: sanitize_ident(&f.name),
                kind: f.kind,
                init: f.init.clone(),
                occurs: f.occurs,
                edit: f.edit.clone(),
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
        let groups = model
            .groups
            .iter()
            .map(|g| (g.name.clone(), g.members.clone()))
            .collect();
        // Párrafos en orden, con su nombre de método único.
        let mut seen: HashMap<String, u32> = HashMap::new();
        let paragraphs = ir
            .procedures
            .iter()
            .map(|proc| {
                let base = paragraph_method(&proc.name);
                let n = seen.entry(base.clone()).or_insert(0);
                let method = if *n > 0 { format!("{base}_{n}") } else { base };
                *n += 1;
                (proc.name.to_uppercase(), method)
            })
            .collect();
        let files = ir
            .files
            .iter()
            .map(|f| FileSym {
                cobol: f.name.clone(),
                ident: format!("file_{}", sanitize_ident(&f.name)),
                path: f.path.clone(),
                record: f.record.clone(),
            })
            .collect();
        Self {
            fields,
            by_name,
            conditions,
            groups,
            paragraphs,
            files,
        }
    }

    /// Busca un fichero por su nombre COBOL.
    pub(crate) fn file(&self, name: &str) -> Option<&FileSym> {
        let up = name.to_uppercase();
        self.files.iter().find(|f| f.cobol == up)
    }

    /// Busca el fichero cuyo registro `FD` es `record`.
    pub(crate) fn file_of_record(&self, record: &str) -> Option<&FileSym> {
        let up = record.to_uppercase();
        self.files.iter().find(|f| f.record == up)
    }

    /// Los métodos a llamar para un `PERFORM name [THRU thru]`: el
    /// rango de párrafos desde `name` hasta `thru` inclusive.
    pub(crate) fn paragraph_range(&self, name: &str, thru: Option<&str>) -> Vec<String> {
        let up = name.to_uppercase();
        let Some(start) = self.paragraphs.iter().position(|(c, _)| *c == up) else {
            return vec![paragraph_method(name)];
        };
        let end = match thru {
            Some(t) => {
                let tu = t.to_uppercase();
                self.paragraphs
                    .iter()
                    .position(|(c, _)| *c == tu)
                    .unwrap_or(start)
            }
            None => start,
        };
        let (lo, hi) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        self.paragraphs[lo..=hi]
            .iter()
            .map(|(_, m)| m.clone())
            .collect()
    }

    /// Los miembros de un grupo, si `name` es un grupo.
    pub(crate) fn group(&self, name: &str) -> Option<&[String]> {
        self.groups.get(&name.to_uppercase()).map(|v| v.as_slice())
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
