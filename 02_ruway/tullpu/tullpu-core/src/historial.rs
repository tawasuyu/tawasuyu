//! Pila de undo/redo genérica con *coalescing* por etiqueta.
//!
//! Motor agnóstico de GUI (regla #2): extraído del `Model` de
//! `tullpu-app-llimphi`, donde vivía como tres campos sueltos (`historial`,
//! `cursor_historial`, `ultima_etiqueta_snapshot`) más las funciones de
//! `historial.rs`. Guarda snapshots del estado vivo `T` (típicamente un
//! [`crate::Lienzo`]); el estado vivo en sí vive afuera y se le pasa a
//! [`Historial::pushear`] para snapshotearlo, y se restaura desde lo que
//! devuelven [`Historial::deshacer`]/[`Historial::rehacer`].

use uuid::Uuid;

/// Etiqueta de un snapshot: capa afectada (`Uuid`) + categoría de la
/// operación (`&'static str`). Dos pushes consecutivos con la misma etiqueta
/// estando en el tope se *coalescen* en uno solo — así un drag continuo
/// (decenas de mensajes por segundo) cuenta como una sola operación
/// reversible.
pub type Etiqueta = (Uuid, &'static str);

/// Pila de snapshots con cursor de navegación y coalescing por etiqueta.
#[derive(Debug, Clone)]
pub struct Historial<T> {
    estados: Vec<T>,
    cursor: usize,
    ultima_etiqueta: Option<Etiqueta>,
    cap: usize,
}

impl<T: Clone> Historial<T> {
    /// Arranca con un único estado base. `cap` es el máximo de entradas
    /// retenidas (las más viejas desfilan por el frente).
    pub fn nuevo(inicial: T, cap: usize) -> Self {
        Self {
            estados: vec![inicial],
            cursor: 0,
            ultima_etiqueta: None,
            cap,
        }
    }

    /// Cantidad de snapshots retenidos.
    pub fn len(&self) -> usize {
        self.estados.len()
    }

    /// Siempre hay al menos el estado base — nunca está vacío.
    pub fn is_empty(&self) -> bool {
        self.estados.is_empty()
    }

    /// Índice del snapshot vigente (el que cuadra con el estado vivo en
    /// régimen estable).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Etiqueta del último push (para inspección/tests).
    pub fn ultima_etiqueta(&self) -> Option<Etiqueta> {
        self.ultima_etiqueta
    }

    /// `true` si hay un estado anterior al que volver.
    pub fn puede_deshacer(&self) -> bool {
        self.cursor > 0
    }

    /// `true` si hay una rama de redo por delante.
    pub fn puede_rehacer(&self) -> bool {
        self.cursor + 1 < self.estados.len()
    }

    /// Corta el coalesce: el próximo push arranca una entrada nueva aunque
    /// traiga la misma etiqueta que el anterior.
    pub fn invalidar_etiqueta(&mut self) {
        self.ultima_etiqueta = None;
    }

    /// Resetea el historial dejando `inicial` como único estado base.
    pub fn reiniciar(&mut self, inicial: T) {
        self.estados = vec![inicial];
        self.cursor = 0;
        self.ultima_etiqueta = None;
    }

    /// Snapshotea `vivo` (el estado actual). Si la `etiqueta` coincide con
    /// la del último snapshot Y estamos en el tope, sustituye en lugar de
    /// agregar (coalesce de drags). Si no, trunca cualquier rama de redo y
    /// agrega; capado a `cap` (las entradas viejas desfilan, el cursor baja
    /// con ellas).
    pub fn pushear(&mut self, vivo: &T, etiqueta: Option<Etiqueta>) {
        let en_el_tope = self.cursor + 1 == self.estados.len();
        let coalesce = match (self.ultima_etiqueta, etiqueta) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
        if en_el_tope && coalesce {
            self.estados[self.cursor] = vivo.clone();
        } else {
            self.estados.truncate(self.cursor + 1);
            self.estados.push(vivo.clone());
            while self.estados.len() > self.cap {
                self.estados.remove(0);
            }
            self.cursor = self.estados.len() - 1;
        }
        self.ultima_etiqueta = etiqueta;
    }

    /// Retrocede el cursor un paso y devuelve el estado a restaurar, o
    /// `None` si ya estás en el base. Invalida la etiqueta (cualquier
    /// mutación posterior arranca rama nueva).
    pub fn deshacer(&mut self) -> Option<&T> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        self.ultima_etiqueta = None;
        Some(&self.estados[self.cursor])
    }

    /// Avanza el cursor un paso (reaplica un estado deshecho) y devuelve el
    /// estado a restaurar, o `None` si no hay rama de redo.
    pub fn rehacer(&mut self) -> Option<&T> {
        if self.cursor + 1 >= self.estados.len() {
            return None;
        }
        self.cursor += 1;
        self.ultima_etiqueta = None;
        Some(&self.estados[self.cursor])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn et(cat: &'static str) -> Option<Etiqueta> {
        Some((Uuid::nil(), cat))
    }

    #[test]
    fn push_distinto_agrega_y_undo_redo_navegan() {
        let mut h = Historial::nuevo(0_i32, 64);
        h.pushear(&1, et("a"));
        h.pushear(&2, et("b"));
        assert_eq!(h.len(), 3);
        assert_eq!(h.cursor(), 2);
        assert_eq!(h.deshacer().copied(), Some(1));
        assert_eq!(h.deshacer().copied(), Some(0));
        assert_eq!(h.deshacer(), None);
        assert_eq!(h.rehacer().copied(), Some(1));
    }

    #[test]
    fn misma_etiqueta_en_el_tope_coalesce() {
        let mut h = Historial::nuevo(0_i32, 64);
        h.pushear(&1, et("drag"));
        h.pushear(&2, et("drag")); // misma etiqueta en el tope ⇒ sustituye
        assert_eq!(h.len(), 2);
        assert_eq!(h.deshacer().copied(), Some(0)); // un solo undo deshace el drag
    }

    #[test]
    fn mutacion_tras_undo_trunca_redo() {
        let mut h = Historial::nuevo(0_i32, 64);
        h.pushear(&1, et("a"));
        h.pushear(&2, et("b"));
        h.deshacer(); // cursor → 1
        h.pushear(&9, et("c")); // trunca el 2
        assert_eq!(h.len(), 3);
        assert!(!h.puede_rehacer());
    }

    #[test]
    fn cap_desfila_las_viejas() {
        let mut h = Historial::nuevo(0_i32, 4);
        for k in 1..=10 {
            h.pushear(&k, None); // sin etiqueta ⇒ nunca coalesce
        }
        assert_eq!(h.len(), 4);
        assert_eq!(h.cursor(), 3);
    }
}
