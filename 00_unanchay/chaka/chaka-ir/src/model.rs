//! El modelo de datos resuelto: el árbol de `DataItem` aplanado a una
//! lista de campos elementales y a los nombres de condición (nivel 88).
//!
//! Es la fuente única de verdad sobre «qué tipo de campo describe una
//! PICTURE» — `charka-codegen` y `charka-shadow` la consumen en vez de
//! reimplementar cada uno la clasificación.

use charka_bcd::{Decimal, Picture};
use charka_parser::DataItem;

use crate::ast::Operand;

/// El tipo resuelto de un dato elemental.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// Numérico: dígitos enteros, fraccionarios y si lleva signo.
    Num { int: u8, frac: u8, signed: bool },
    /// Alfanumérico de longitud fija.
    Text { len: usize },
}

/// Un dato elemental del programa, listo para materializarse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Nombre COBOL, en mayúsculas.
    pub name: String,
    /// Numérico o alfanumérico.
    pub kind: FieldKind,
    /// Valor inicial ya normalizado (de la cláusula `VALUE`).
    pub init: String,
}

/// Un nombre de condición — un dato de nivel 88. `IF <name>` equivale
/// a comparar `parent` con `value`.
#[derive(Debug, Clone, PartialEq)]
pub struct ConditionName {
    /// Nombre del 88, en mayúsculas.
    pub name: String,
    /// El dato sobre el que se prueba la condición.
    pub parent: String,
    /// El valor que hace verdadera la condición.
    pub value: Operand,
}

/// El modelo de datos resuelto de un programa.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataModel {
    /// Los datos elementales, en orden de declaración.
    pub fields: Vec<Field>,
    /// Los nombres de condición (nivel 88).
    pub conditions: Vec<ConditionName>,
}

impl DataModel {
    /// Busca un campo por su nombre COBOL (sin distinguir mayúsculas).
    pub fn field(&self, name: &str) -> Option<&Field> {
        let up = name.to_uppercase();
        self.fields.iter().find(|f| f.name == up)
    }

    /// Busca un nombre de condición.
    pub fn condition(&self, name: &str) -> Option<&ConditionName> {
        let up = name.to_uppercase();
        self.conditions.iter().find(|c| c.name == up)
    }
}

/// Aplana el árbol de datos en un [`DataModel`].
pub fn resolve_data(data: &[DataItem]) -> DataModel {
    let mut model = DataModel::default();
    walk(data, &mut model);
    model
}

/// Recorre el árbol: registra los 88 como condiciones sobre su dato
/// padre, recurre en los grupos y emite los datos elementales.
fn walk(items: &[DataItem], model: &mut DataModel) {
    for it in items {
        if it.level == 66 || it.level == 88 {
            // Los 88 los registra su dato padre; los 66 se omiten.
            continue;
        }
        // Los hijos de nivel 88 son condiciones sobre este dato.
        for child in &it.children {
            if child.level == 88 {
                model.conditions.push(ConditionName {
                    name: child.name.to_uppercase(),
                    parent: it.name.to_uppercase(),
                    value: condition_value(child.value.as_deref()),
                });
            }
        }
        // Un dato con hijos «reales» (no 88/66) es un grupo.
        let is_group = it.children.iter().any(|c| c.level != 88 && c.level != 66);
        if is_group {
            walk(&it.children, model);
        } else if it.name != "FILLER" {
            if let Some(kind) = classify(it.picture.as_deref()) {
                let init = match kind {
                    FieldKind::Num { .. } => numeric_value(it.value.as_deref()),
                    FieldKind::Text { .. } => text_value(it.value.as_deref()),
                };
                model.fields.push(Field {
                    name: it.name.to_uppercase(),
                    kind,
                    init,
                });
            }
        }
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
    if let Ok(p) = Picture::parse(&up) {
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

/// Normaliza el `VALUE` de un campo numérico a un literal parseable.
fn numeric_value(v: Option<&str>) -> String {
    let Some(raw) = v else {
        return "0".to_string();
    };
    if matches!(raw.to_uppercase().as_str(), "ZERO" | "ZEROS" | "ZEROES") {
        return "0".to_string();
    }
    if Decimal::parse(raw).is_ok() {
        raw.to_string()
    } else {
        "0".to_string()
    }
}

/// Normaliza el `VALUE` de un campo de texto. El parser envuelve los
/// literales de texto en comillas simples; aquí se desenvuelven.
fn text_value(v: Option<&str>) -> String {
    let Some(raw) = v else {
        return String::new();
    };
    let up = raw.to_uppercase();
    if matches!(up.as_str(), "SPACE" | "SPACES") {
        return String::new();
    }
    if matches!(up.as_str(), "ZERO" | "ZEROS" | "ZEROES") {
        return "0".to_string();
    }
    if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        raw[1..raw.len() - 1].to_string()
    } else {
        raw.to_string()
    }
}

/// El valor de un nivel 88 como [`Operand`]: literal de texto entre
/// comillas, número, o (si no es ninguno) texto crudo.
fn condition_value(value: Option<&str>) -> Operand {
    let Some(raw) = value else {
        return Operand::Num("0".to_string());
    };
    if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        return Operand::Str(raw[1..raw.len() - 1].to_string());
    }
    if Decimal::parse(raw).is_ok() {
        Operand::Num(raw.to_string())
    } else {
        Operand::Str(raw.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use charka_lexer::{lex, SourceFormat};

    fn model_of(src: &str) -> DataModel {
        let toks = lex(src, SourceFormat::Free).unwrap();
        let program = charka_parser::parse(&toks).unwrap();
        resolve_data(&program.data)
    }

    #[test]
    fn flattens_elementary_fields() {
        let m = model_of(
            "DATA DIVISION.\n\
             01 WS-N PIC 9(3) VALUE 7.\n\
             01 WS-T PIC X(4) VALUE 'AB'.\n",
        );
        assert_eq!(m.fields.len(), 2);
        assert_eq!(
            m.field("WS-N").unwrap().kind,
            FieldKind::Num {
                int: 3,
                frac: 0,
                signed: false
            }
        );
        assert_eq!(m.field("WS-N").unwrap().init, "7");
        assert_eq!(m.field("WS-T").unwrap().kind, FieldKind::Text { len: 4 });
        assert_eq!(m.field("WS-T").unwrap().init, "AB");
    }

    #[test]
    fn group_items_are_not_fields_but_their_children_are() {
        let m = model_of(
            "DATA DIVISION.\n\
             01 WS-REC.\n\
                05 WS-A PIC 9(2).\n\
                05 WS-B PIC X(3).\n",
        );
        assert!(m.field("WS-REC").is_none());
        assert!(m.field("WS-A").is_some());
        assert!(m.field("WS-B").is_some());
    }

    #[test]
    fn level_88_becomes_a_condition_on_its_parent() {
        let m = model_of(
            "DATA DIVISION.\n\
             01 WS-FLAG PIC X VALUE 'N'.\n\
                88 ES-SI VALUE 'Y'.\n\
                88 ES-NO VALUE 'N'.\n",
        );
        // El dato con hijos 88 sigue siendo un campo.
        assert!(m.field("WS-FLAG").is_some());
        let si = m.condition("ES-SI").unwrap();
        assert_eq!(si.parent, "WS-FLAG");
        assert_eq!(si.value, Operand::Str("Y".into()));
        assert_eq!(
            m.condition("ES-NO").unwrap().value,
            Operand::Str("N".into())
        );
    }

    #[test]
    fn numeric_level_88_value() {
        let m = model_of(
            "DATA DIVISION.\n\
             01 WS-COD PIC 9(2) VALUE 0.\n\
                88 ES-OK VALUE 0.\n",
        );
        assert_eq!(
            m.condition("ES-OK").unwrap().value,
            Operand::Num("0".into())
        );
    }
}
