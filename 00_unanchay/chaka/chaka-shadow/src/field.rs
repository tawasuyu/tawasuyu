//! El estado de los datos durante la ejecución sombra: el modelo de
//! datos resuelto de `chaka-ir` se materializa en campos vivos.

use std::collections::HashMap;

use chaka_ir::{DataModel, FieldKind};
use chaka_runtime::{Num, Picture, Text};

/// Un campo vivo. Todo campo es un vector: un dato escalar es un
/// vector de un elemento; una tabla (`OCCURS n`) es de `n` elementos.
pub(crate) enum Cell {
    Num(Vec<Num>),
    Text(Vec<Text>),
}

/// Materializa los campos del modelo en un mapa `nombre → campo`.
pub(crate) fn build_fields(model: &DataModel) -> HashMap<String, Cell> {
    let mut map = HashMap::new();
    for f in &model.fields {
        let n = f.occurs.unwrap_or(1).max(1) as usize;
        let cell = match f.kind {
            FieldKind::Num { int, frac, signed } => Cell::Num(vec![
                Num::with_value(
                    Picture::new(int, frac, signed),
                    &f.init
                );
                n
            ]),
            FieldKind::Text { len } => Cell::Text(vec![Text::with_value(len, &f.init); n]),
        };
        map.entry(f.name.clone()).or_insert(cell);
    }
    map
}
