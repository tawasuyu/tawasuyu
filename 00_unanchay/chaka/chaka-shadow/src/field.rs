//! El estado de los datos durante la ejecución sombra: el árbol de
//! `DataItem` del IR se aplana a un mapa de campos vivos.
//!
//! La clasificación de PICTURE refleja la de `charka-codegen` — un
//! futuro refactor la unificaría en `charka-runtime`.

use std::collections::HashMap;

use charka_ir::DataItem;
use charka_runtime::{Num, Picture, Text};

/// Un campo vivo: numérico o alfanumérico.
pub(crate) enum Cell {
    Num(Num),
    Text(Text),
}

/// Aplana el árbol de datos en un mapa `nombre COBOL → campo`.
pub(crate) fn build_fields(data: &[DataItem]) -> HashMap<String, Cell> {
    let mut map = HashMap::new();
    collect(data, &mut map);
    map
}

/// Recorre el árbol: los grupos no son campos (se recurre en sus
/// hijos); se saltan los niveles 88/66 y los `FILLER`.
fn collect(items: &[DataItem], map: &mut HashMap<String, Cell>) {
    for it in items {
        if it.level == 88 || it.level == 66 {
            continue;
        }
        if !it.children.is_empty() {
            collect(&it.children, map);
            continue;
        }
        if it.name == "FILLER" {
            continue;
        }
        if let Some(cell) = make_cell(it.picture.as_deref(), it.value.as_deref()) {
            map.entry(it.name.to_uppercase()).or_insert(cell);
        }
    }
}

/// Construye un campo desde su PICTURE y su cláusula `VALUE`.
fn make_cell(pic: Option<&str>, value: Option<&str>) -> Option<Cell> {
    let up = pic?.to_uppercase();
    if up.contains('X') || up.contains('A') {
        return Some(Cell::Text(Text::with_value(
            pic_width(&up).max(1),
            &text_value(value),
        )));
    }
    if let Ok(p) = Picture::parse(&up) {
        return Some(Cell::Num(Num::with_value(p, &numeric_value(value))));
    }
    // PICTURE de edición → campo de texto de presentación.
    Some(Cell::Text(Text::with_value(
        pic_width(&up).max(1),
        &text_value(value),
    )))
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
    if charka_runtime::Decimal::parse(raw).is_ok() {
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
