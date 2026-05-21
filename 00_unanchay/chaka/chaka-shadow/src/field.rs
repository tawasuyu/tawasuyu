//! El estado de los datos durante la ejecución sombra: el modelo de
//! datos resuelto de `charka-ir` se materializa en campos vivos.

use std::collections::HashMap;

use charka_ir::{DataModel, FieldKind};
use charka_runtime::{Num, Picture, Text};

/// Un campo vivo: numérico o alfanumérico.
pub(crate) enum Cell {
    Num(Num),
    Text(Text),
}

/// Materializa los campos del modelo en un mapa `nombre → campo`.
pub(crate) fn build_fields(model: &DataModel) -> HashMap<String, Cell> {
    let mut map = HashMap::new();
    for f in &model.fields {
        let cell = match f.kind {
            FieldKind::Num { int, frac, signed } => {
                Cell::Num(Num::with_value(Picture::new(int, frac, signed), &f.init))
            }
            FieldKind::Text { len } => Cell::Text(Text::with_value(len, &f.init)),
        };
        map.entry(f.name.clone()).or_insert(cell);
    }
    map
}
