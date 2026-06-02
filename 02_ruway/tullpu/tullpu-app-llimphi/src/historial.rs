//! Pila de undo/redo de la app `tullpu`: snapshots del `Lienzo` con
//! coalescing por etiqueta, navegación del cursor de historial y reajuste
//! de la selección tras restaurar un estado.
//!
//! Behavior-preserving split de `main.rs` — sin cambios funcionales.

use uuid::Uuid;

use crate::model::*;

/// Pushea el estado actual del lienzo a la pila de undo. Si la `etiqueta`
/// (Uuid de capa + categoría) coincide con la del último snapshot Y estamos
/// en el tope, sustituye en lugar de pushear — es el mecanismo de *coalesce*
/// para drags continuos (slider de opacidad disparando decenas de mensajes
/// por segundo). Si no, trunca la rama de redo y agrega entrada nueva.
///
/// Se invoca después de cualquier mutación de `model.lienzo` que el usuario
/// pueda querer revertir (toggle visible, blend, opacidad, mover, dup, elim,
/// agregar, rename, file drop). Las acciones de pura UI (Seleccionar,
/// Recargar, Exportar, Picker abrir/cerrar) no producen snapshot.
pub(crate) fn pushear_snapshot(model: &mut Model, etiqueta: Option<(Uuid, &'static str)>) {
    model.hist.pushear(&model.lienzo, etiqueta);
}

/// Restaura el estado anterior del lienzo (cursor−−). Devuelve `true` si hubo
/// algo que deshacer. El almacén content-addressed nunca borra buffers, así
/// que restaurar a una versión anterior siempre encuentra los hashes — los
/// buffers "huérfanos" de la versión actual quedan dormidos pero accesibles
/// si después se hace redo. Recomposición posterior a cargo del caller.
pub(crate) fn aplicar_undo(model: &mut Model) -> bool {
    match model.hist.deshacer() {
        Some(l) => {
            model.lienzo = l.clone();
            true
        }
        None => false,
    }
}

/// Reaplica un estado del que ya habíamos hecho undo (cursor++).
pub(crate) fn aplicar_redo(model: &mut Model) -> bool {
    match model.hist.rehacer() {
        Some(l) => {
            model.lienzo = l.clone();
            true
        }
        None => false,
    }
}

/// Tras restaurar `model.lienzo` desde el historial, la selección puede
/// apuntar a una capa que no existe en ese estado (ej. la creé, le hice
/// Eliminar, ahora Ctrl+Z trae de vuelta una versión ANTERIOR a la creación).
/// Si la seleccionada ya no está, caemos al tope visual del lienzo restaurado.
pub(crate) fn ajustar_seleccion_tras_restaurar(model: &mut Model) {
    let existe = model
        .seleccionada
        .map(|id| model.lienzo.capas.iter().any(|c| c.id == id))
        .unwrap_or(false);
    if !existe {
        model.seleccionada = model.lienzo.capas.last().map(|c| c.id);
    }
}
