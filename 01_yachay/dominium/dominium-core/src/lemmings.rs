//! Los Agentes Vectoriales — Lemmings en Structure-of-Arrays.
//!
//! Sin objetos ni punteros por agente: vectores paralelos indexados por
//! un `usize` continuo. Datos crudos alineados en caché.

use serde::{Deserialize, Serialize};

/// Índices de las cuatro componentes de `vector_psi`.
pub const PSI_ORDEN: usize = 0;
pub const PSI_MIEDO: usize = 1;
pub const PSI_CURIOSIDAD: usize = 2;
pub const PSI_CORRUPTIBILIDAD: usize = 3;

/// Población de Lemmings en SoA. Todos los vectores tienen el mismo largo.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Lemmings {
    pub pos_x: Vec<f32>,
    pub pos_y: Vec<f32>,
    /// Contador incremental de ticks de vida.
    pub edad: Vec<u32>,
    /// Escalar de salud; si llega a 0 el agente muere.
    pub energia: Vec<f32>,
    /// Tensores de sesgo interno `[Orden, Miedo, Curiosidad, Corruptibilidad]`.
    pub vector_psi: Vec<[f32; 4]>,
    /// Byte discriminador de la máquina de estados (0-5).
    pub accion: Vec<u8>,
}

impl Lemmings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.pos_x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pos_x.is_empty()
    }

    /// Instancia un Lemming nuevo (edad 0). Devuelve su índice.
    pub fn spawn(&mut self, x: f32, y: f32, energia: f32, psi: [f32; 4]) -> usize {
        let i = self.len();
        self.pos_x.push(x);
        self.pos_y.push(y);
        self.edad.push(0);
        self.energia.push(energia);
        self.vector_psi.push(psi);
        self.accion.push(0);
        i
    }

    /// Elimina el Lemming `i` por `swap_remove` — O(1), no preserva el
    /// orden (el último ocupa el hueco).
    pub fn remove(&mut self, i: usize) {
        self.pos_x.swap_remove(i);
        self.pos_y.swap_remove(i);
        self.edad.swap_remove(i);
        self.energia.swap_remove(i);
        self.vector_psi.swap_remove(i);
        self.accion.swap_remove(i);
    }

    /// Distancia euclidiana al cuadrado entre dos Lemmings (sin `sqrt` —
    /// suficiente para comparar cercanía y bit-exacto).
    pub fn dist2(&self, a: usize, b: usize) -> f32 {
        let dx = self.pos_x[a] - self.pos_x[b];
        let dy = self.pos_y[a] - self.pos_y[b];
        dx * dx + dy * dy
    }

    /// Índice del Lemming vivo más cercano a `i` (distinto de `i`), o
    /// `None` si es el único. Determinista: ante empate gana el menor
    /// índice.
    pub fn nearest(&self, i: usize) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for j in 0..self.len() {
            if j == i {
                continue;
            }
            let d = self.dist2(i, j);
            if best.map(|(_, bd)| d < bd).unwrap_or(true) {
                best = Some((j, d));
            }
        }
        best.map(|(j, _)| j)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_and_remove() {
        let mut l = Lemmings::new();
        let a = l.spawn(1.0, 1.0, 10.0, [0.0; 4]);
        let _b = l.spawn(2.0, 2.0, 20.0, [0.0; 4]);
        assert_eq!((a, l.len()), (0, 2));
        l.remove(a);
        assert_eq!(l.len(), 1);
        // swap_remove: el agente "b" ocupa el índice 0.
        assert_eq!(l.energia[0], 20.0);
    }

    #[test]
    fn nearest_picks_closest_and_breaks_ties_by_index() {
        let mut l = Lemmings::new();
        l.spawn(0.0, 0.0, 1.0, [0.0; 4]); // 0
        l.spawn(10.0, 0.0, 1.0, [0.0; 4]); // 1 — lejos
        l.spawn(1.0, 0.0, 1.0, [0.0; 4]); // 2 — cerca de 0
        assert_eq!(l.nearest(0), Some(2));
        // Único agente → None.
        let mut solo = Lemmings::new();
        solo.spawn(0.0, 0.0, 1.0, [0.0; 4]);
        assert_eq!(solo.nearest(0), None);
    }
}
