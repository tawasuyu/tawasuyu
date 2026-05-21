//! Tabla de símbolos: el modelo de datos COBOL traducido a campos Rust.

use std::collections::HashMap;

use charka_ir::DataItem;

/// Un campo del struct `Program` generado.
pub(crate) struct Field {
    /// Nombre COBOL en mayúsculas.
    pub cobol: String,
    /// Identificador Rust saneado y único.
    pub ident: String,
    /// Numérico o alfanumérico.
    pub kind: FieldKind,
    /// La cláusula `VALUE`, si la hay.
    pub value: Option<String>,
}

/// El tipo de un campo elemental.
pub(crate) enum FieldKind {
    /// Campo numérico — se emite como `Num`.
    Num { int: u8, frac: u8, signed: bool },
    /// Campo alfanumérico — se emite como `Text`.
    Text { len: usize },
}

/// El conjunto de campos del programa, indexado por nombre COBOL.
pub(crate) struct Symbols {
    pub fields: Vec<Field>,
    by_name: HashMap<String, usize>,
}

impl Symbols {
    /// Construye la tabla recorriendo el árbol de datos.
    pub(crate) fn build(data: &[DataItem]) -> Self {
        let mut fields = Vec::new();
        collect(data, &mut fields);
        dedup_idents(&mut fields);
        let by_name = fields
            .iter()
            .enumerate()
            .map(|(i, f)| (f.cobol.clone(), i))
            .collect();
        Self { fields, by_name }
    }

    /// Busca un campo por su nombre COBOL (sin distinguir mayúsculas).
    pub(crate) fn lookup(&self, cobol: &str) -> Option<&Field> {
        self.by_name
            .get(&cobol.to_uppercase())
            .map(|&i| &self.fields[i])
    }
}

/// Recoge los datos elementales del árbol. Los grupos no son campos —
/// se recurre en sus hijos. Se saltan niveles 88/66 y los `FILLER`.
fn collect(items: &[DataItem], out: &mut Vec<Field>) {
    for it in items {
        if it.level == 88 || it.level == 66 {
            continue;
        }
        if !it.children.is_empty() {
            collect(&it.children, out);
            continue;
        }
        if it.name == "FILLER" {
            continue;
        }
        let Some(kind) = classify(it.picture.as_deref()) else {
            continue;
        };
        out.push(Field {
            cobol: it.name.clone(),
            ident: sanitize_ident(&it.name),
            kind,
            value: it.value.clone(),
        });
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

/// Clasifica una cláusula PICTURE: alfanumérica si tiene `X`/`A`,
/// numérica si `charka-bcd` la parsea; una PICTURE de edición se trata
/// como texto de presentación.
fn classify(pic: Option<&str>) -> Option<FieldKind> {
    let up = pic?.to_uppercase();
    if up.contains('X') || up.contains('A') {
        return Some(FieldKind::Text {
            len: pic_width(&up).max(1),
        });
    }
    if let Ok(p) = charka_bcd::Picture::parse(&up) {
        return Some(FieldKind::Num {
            int: p.integer_digits,
            frac: p.fraction_digits,
            signed: p.signed,
        });
    }
    Some(FieldKind::Text {
        len: pic_width(&up).max(1),
    })
}

/// Cuenta las posiciones de presentación de una PICTURE, expandiendo
/// la repetición `C(n)`. `S` y `V` no ocupan posición.
fn pic_width(up: &str) -> usize {
    let chars: Vec<char> = up.chars().collect();
    let mut i = 0;
    let mut total = 0usize;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        if c == 'S' || c == 'V' {
            continue;
        }
        let mut count = 1usize;
        if chars.get(i) == Some(&'(') {
            i += 1;
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if let Ok(n) = chars[start..i].iter().collect::<String>().parse::<usize>() {
                count = n;
            }
            if chars.get(i) == Some(&')') {
                i += 1;
            }
        }
        total += count;
    }
    total
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
