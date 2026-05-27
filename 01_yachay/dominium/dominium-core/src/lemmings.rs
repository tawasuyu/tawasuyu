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
/// Quinta componente *opcional* del psi — la dimensión de Extraversión del
/// modelo Big Five. Mapea a sociabilidad / asertividad / energía social.
/// Vive en su propio `Vec<f32>` (`Lemmings::psi5`) en lugar de extender el
/// `vector_psi` a `[f32; 5]` para preservar bit-exactitud y serde compat con
/// motores Big Four históricos.
pub const PSI_EXTRAVERSION: usize = 4;

/// Valor default de `psi5` cuando se hace `spawn` sin especificarlo o cuando
/// un `World` antiguo se deserializa sin la columna. Elegimos 0.5 para que
/// sea "ni introvertido ni extravertido" y la psicología quede neutral.
pub const PSI_EXTRAVERSION_DEFAULT: f32 = 0.5;

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
    /// Ticks restantes de captura por un `BehaviorHack` de un Concepto.
    /// Mientras es > 0, el Lemming ejecuta su `accion` sin reevaluar
    /// transiciones (la captura sobrescribe a la desesperación).
    pub hack_lock: Vec<u32>,
    /// Quinta dimensión opcional del psi — Big Five Extraversion. Cuando el
    /// motor corre en modo Big Four (`SimParams::big_five == false`), este
    /// vector se mantiene poblado con el default `PSI_EXTRAVERSION_DEFAULT`
    /// pero no afecta ninguna ecuación. Saves históricos sin esta columna
    /// vienen vacíos y se rellenan vía [`Lemmings::ensure_psi5_len`].
    #[serde(default)]
    pub psi5: Vec<f32>,
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

    /// Instancia un Lemming nuevo (edad 0). Devuelve su índice. La quinta
    /// componente del psi (Big Five Extraversion) queda en el default neutral
    /// `PSI_EXTRAVERSION_DEFAULT`; usar [`Lemmings::spawn_big5`] para fijarla.
    pub fn spawn(&mut self, x: f32, y: f32, energia: f32, psi: [f32; 4]) -> usize {
        self.spawn_big5(x, y, energia, psi, PSI_EXTRAVERSION_DEFAULT)
    }

    /// Como [`spawn`], pero pone explícitamente el quinto componente `psi5`
    /// (Big Five Extraversion). Usar cuando el motor corre con
    /// `SimParams::big_five = true` y los agentes nacen con una distribución
    /// de extraversión no trivial.
    pub fn spawn_big5(
        &mut self,
        x: f32,
        y: f32,
        energia: f32,
        psi: [f32; 4],
        psi5: f32,
    ) -> usize {
        let i = self.len();
        self.pos_x.push(x);
        self.pos_y.push(y);
        self.edad.push(0);
        self.energia.push(energia);
        self.vector_psi.push(psi);
        self.accion.push(0);
        self.hack_lock.push(0);
        self.psi5.push(psi5);
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
        self.hack_lock.swap_remove(i);
        // El `psi5` de saves Big Four puede estar vacío — sólo recortamos si
        // hay algo. Mantiene la invariante "len == pos_x.len() ∨ len == 0".
        if !self.psi5.is_empty() {
            self.psi5.swap_remove(i);
        }
    }

    /// Asegura que `psi5` tenga el mismo largo que `pos_x`, rellenando con
    /// `PSI_EXTRAVERSION_DEFAULT` lo que falte. Idempotente. Sirve para
    /// "ascender" saves Big Four a Big Five sin perder la población vieja.
    pub fn ensure_psi5_len(&mut self) {
        let n = self.pos_x.len();
        if self.psi5.len() < n {
            self.psi5.resize(n, PSI_EXTRAVERSION_DEFAULT);
        } else if self.psi5.len() > n {
            self.psi5.truncate(n);
        }
    }

    /// Lectura segura del quinto componente. Cuando `psi5` está vacío
    /// (saves históricos Big Four) devuelve `PSI_EXTRAVERSION_DEFAULT`; con
    /// `i` fuera de rango, también — usar sólo con índices válidos.
    pub fn psi5_at(&self, i: usize) -> f32 {
        self.psi5.get(i).copied().unwrap_or(PSI_EXTRAVERSION_DEFAULT)
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

    /// Índice del Lemming vivo con **menor energía** distinto de `i`. Es
    /// el destinatario de `act_intercambiar` cuando la estrategia es
    /// "redistribución solidaria": en lugar de donar al vecino físico
    /// más cercano (que puede ser igualmente pobre), busca al más
    /// necesitado del mundo. Determinista: ante empate, menor índice.
    pub fn poorest(&self, i: usize) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for j in 0..self.len() {
            if j == i {
                continue;
            }
            let e = self.energia[j];
            if best.map(|(_, be)| e < be).unwrap_or(true) {
                best = Some((j, e));
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
    fn spawn_default_pone_psi5_neutral() {
        let mut l = Lemmings::new();
        let i = l.spawn(1.0, 1.0, 10.0, [0.5; 4]);
        assert_eq!(l.psi5.len(), l.pos_x.len());
        assert_eq!(l.psi5_at(i), PSI_EXTRAVERSION_DEFAULT);
    }

    #[test]
    fn spawn_big5_fija_psi5_explicito() {
        let mut l = Lemmings::new();
        let i = l.spawn_big5(0.0, 0.0, 10.0, [0.5; 4], 0.9);
        assert!((l.psi5_at(i) - 0.9).abs() < 1e-6);
    }

    #[test]
    fn ensure_psi5_len_completa_columna_faltante() {
        // Simula un save Big Four cargado por serde sin la columna psi5.
        let mut l = Lemmings {
            pos_x: vec![1.0, 2.0, 3.0],
            pos_y: vec![1.0, 2.0, 3.0],
            edad: vec![0; 3],
            energia: vec![10.0; 3],
            vector_psi: vec![[0.5; 4]; 3],
            accion: vec![0; 3],
            hack_lock: vec![0; 3],
            psi5: Vec::new(),
        };
        l.ensure_psi5_len();
        assert_eq!(l.psi5.len(), 3);
        for v in &l.psi5 {
            assert!((*v - PSI_EXTRAVERSION_DEFAULT).abs() < 1e-6);
        }
    }

    #[test]
    fn remove_actualiza_psi5() {
        let mut l = Lemmings::new();
        l.spawn_big5(0.0, 0.0, 10.0, [0.0; 4], 0.1);
        l.spawn_big5(0.0, 0.0, 10.0, [0.0; 4], 0.9);
        assert_eq!(l.psi5, vec![0.1, 0.9]);
        l.remove(0);
        // swap_remove deja el último (0.9) en el hueco 0.
        assert_eq!(l.psi5, vec![0.9]);
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
