//! El mundo: grilla + lemmings, y las 6 acciones atómicas fijas.
//!
//! Cualquier "profesión" o "rol" del macro es sólo un Lemming ejecutando
//! una de estas 6 acciones en un entorno específico.

use crate::conceptos::Conceptos;
use crate::grid::Grid;
use crate::lemmings::{Lemmings, PSI_CORRUPTIBILIDAD, PSI_CURIOSIDAD, PSI_MIEDO, PSI_ORDEN};
use crate::params::SimParams;
use serde::{Deserialize, Serialize};

/// Las 6 acciones atómicas. El byte `accion` del Lemming es uno de estos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Action {
    /// Lee gradientes vecinos, se mueve hacia el óptimo, gasta energía.
    Mover = 0,
    /// Resta de la celda actual, suma a su energía, degrada el suelo.
    Extraer = 1,
    /// Acerca su `vector_psi` a los campos de la celda actual.
    Sincronizar = 2,
    /// Transfiere energía al vecino más cercano.
    Intercambiar = 3,
    /// Gasta energía para instanciar un Lemming hijo (edad 0).
    Replicar = 4,
    /// Resta energía al vecino más cercano y absorbe una fracción.
    Degradar = 5,
}

impl Action {
    /// Convierte el byte discriminador. `None` si está fuera de rango.
    pub fn from_u8(b: u8) -> Option<Action> {
        match b {
            0 => Some(Action::Mover),
            1 => Some(Action::Extraer),
            2 => Some(Action::Sincronizar),
            3 => Some(Action::Intercambiar),
            4 => Some(Action::Replicar),
            5 => Some(Action::Degradar),
            _ => None,
        }
    }
}

/// El estado completo de la simulación.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct World {
    pub grid: Grid,
    pub lemmings: Lemmings,
    #[serde(default)]
    pub conceptos: Conceptos,
}

impl World {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            grid: Grid::new(width, height),
            lemmings: Lemmings::new(),
            conceptos: Conceptos::new(),
        }
    }

    /// Celda que ocupa el Lemming `i`.
    fn cell_of(&self, i: usize) -> usize {
        let (cx, cy) = self.grid.clamp_cell(self.lemmings.pos_x[i], self.lemmings.pos_y[i]);
        self.grid.idx(cx, cy)
    }

    /// 0 · Mover — gravedad mental hacia el vecino más afín al `vector_psi`.
    pub fn act_mover(&mut self, i: usize, p: &SimParams) {
        let (cx, cy) =
            self.grid.clamp_cell(self.lemmings.pos_x[i], self.lemmings.pos_y[i]);
        let psi = self.lemmings.vector_psi[i];
        let mut best_dir = (0.0f32, 0.0f32);
        let mut best_score = f32::MIN;
        for (dx, dy) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (cx as i64 + dx, cy as i64 + dy);
            if !self.grid.in_bounds(nx, ny) {
                continue;
            }
            let idx = self.grid.idx(nx as usize, ny as usize);
            // Orden busca materia, Miedo evita poder, Curiosidad busca
            // psique, Corruptibilidad busca oro.
            let score = psi[PSI_ORDEN] * self.grid.materia[idx]
                - psi[PSI_MIEDO] * self.grid.poder[idx]
                + psi[PSI_CURIOSIDAD] * self.grid.psique[idx]
                + psi[PSI_CORRUPTIBILIDAD] * self.grid.oro[idx];
            if score > best_score {
                best_score = score;
                best_dir = (dx as f32, dy as f32);
            }
        }
        let w = self.grid.width as f32 - 1.0;
        let h = self.grid.height as f32 - 1.0;
        self.lemmings.pos_x[i] =
            (self.lemmings.pos_x[i] + best_dir.0 * p.move_speed).clamp(0.0, w);
        self.lemmings.pos_y[i] =
            (self.lemmings.pos_y[i] + best_dir.1 * p.move_speed).clamp(0.0, h);
        self.lemmings.energia[i] -= p.move_cost;
    }

    /// 1 · Extraer — vacía materia de la celda hacia la energía del agente.
    pub fn act_extraer(&mut self, i: usize, p: &SimParams) {
        let idx = self.cell_of(i);
        let taken = self.grid.materia[idx].min(p.extract_rate).max(0.0);
        self.grid.materia[idx] -= taken;
        self.lemmings.energia[i] += taken;
        self.grid.degradacion[idx] += p.degr_per_extract;
    }

    /// 2 · Sincronizar — el `vector_psi` deriva hacia los campos de la celda.
    /// Mapeo coherente con `act_mover`: ORDEN↔materia, MIEDO↔poder,
    /// CURIOSIDAD↔psique, CORRUPTIBILIDAD↔oro.
    pub fn act_sincronizar(&mut self, i: usize, p: &SimParams) {
        let idx = self.cell_of(i);
        let mut targets = [0.0f32; 4];
        targets[PSI_ORDEN] = self.grid.materia[idx];
        targets[PSI_MIEDO] = self.grid.poder[idx];
        targets[PSI_CURIOSIDAD] = self.grid.psique[idx];
        targets[PSI_CORRUPTIBILIDAD] = self.grid.oro[idx];
        for k in 0..4 {
            let v = self.lemmings.vector_psi[i][k];
            self.lemmings.vector_psi[i][k] = v + (targets[k] - v) * p.sync_rate;
        }
    }

    /// 3 · Intercambiar — transfiere energía al vecino más cercano.
    pub fn act_intercambiar(&mut self, i: usize, p: &SimParams) {
        let Some(j) = self.lemmings.nearest(i) else { return };
        let amount = p.trade_amount.min(self.lemmings.energia[i]).max(0.0);
        self.lemmings.energia[i] -= amount;
        self.lemmings.energia[j] += amount;
    }

    /// 4 · Replicar — instancia un hijo con edad 0 en las mismas coordenadas.
    pub fn act_replicar(&mut self, i: usize, p: &SimParams) {
        if self.lemmings.energia[i] <= p.replicate_threshold {
            return;
        }
        let cost = self.lemmings.energia[i] * p.child_energy_frac;
        self.lemmings.energia[i] -= cost;
        let (x, y) = (self.lemmings.pos_x[i], self.lemmings.pos_y[i]);
        let psi = self.lemmings.vector_psi[i];
        self.lemmings.spawn(x, y, cost, psi);
    }

    /// 5 · Degradar (Pelear) — resta energía al vecino y absorbe parte.
    pub fn act_degradar(&mut self, i: usize, p: &SimParams) {
        let Some(j) = self.lemmings.nearest(i) else { return };
        let dmg = p.fight_damage.min(self.lemmings.energia[j]).max(0.0);
        self.lemmings.energia[j] -= dmg;
        self.lemmings.energia[i] += dmg * p.absorb_frac;
    }

    /// Despacha la acción del Lemming `i` según su byte `accion`.
    pub fn step_lemming(&mut self, i: usize, p: &SimParams) {
        match Action::from_u8(self.lemmings.accion[i]) {
            Some(Action::Mover) => self.act_mover(i, p),
            Some(Action::Extraer) => self.act_extraer(i, p),
            Some(Action::Sincronizar) => self.act_sincronizar(i, p),
            Some(Action::Intercambiar) => self.act_intercambiar(i, p),
            Some(Action::Replicar) => self.act_replicar(i, p),
            Some(Action::Degradar) => self.act_degradar(i, p),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_1x_lemming() -> (World, SimParams) {
        let mut w = World::new(16, 16);
        w.lemmings.spawn(8.0, 8.0, 100.0, [1.0, 0.0, 0.0, 0.0]);
        (w, SimParams::default())
    }

    #[test]
    fn action_from_u8_covers_0_to_5() {
        for b in 0..=5u8 {
            assert!(Action::from_u8(b).is_some());
        }
        assert!(Action::from_u8(6).is_none());
    }

    #[test]
    fn mover_heads_toward_higher_materia() {
        let (mut w, p) = world_1x_lemming();
        // Materia alta a la derecha de (8,8).
        let right = w.grid.idx(9, 8);
        w.grid.materia[right] = 100.0;
        let x0 = w.lemmings.pos_x[0];
        w.act_mover(0, &p);
        assert!(w.lemmings.pos_x[0] > x0, "se movió hacia la materia");
        assert!(w.lemmings.energia[0] < 100.0, "Mover cuesta energía");
    }

    #[test]
    fn extraer_drains_cell_into_agent_and_degrades() {
        let (mut w, p) = world_1x_lemming();
        let idx = w.grid.idx(8, 8);
        w.grid.materia[idx] = 10.0;
        w.act_extraer(0, &p);
        assert!(w.grid.materia[idx] < 10.0);
        assert!(w.lemmings.energia[0] > 100.0);
        assert!(w.grid.degradacion[idx] > 0.0);
    }

    #[test]
    fn replicar_spawns_child_and_costs_energy() {
        let (mut w, p) = world_1x_lemming(); // energía 100 > umbral 50
        w.act_replicar(0, &p);
        assert_eq!(w.lemmings.len(), 2);
        assert_eq!(w.lemmings.edad[1], 0);
        assert!(w.lemmings.energia[0] < 100.0);
    }

    #[test]
    fn degradar_drains_nearest_and_absorbs() {
        let mut w = World::new(16, 16);
        w.lemmings.spawn(8.0, 8.0, 50.0, [0.0; 4]);
        w.lemmings.spawn(9.0, 8.0, 50.0, [0.0; 4]);
        let p = SimParams::default();
        w.act_degradar(0, &p);
        assert!(w.lemmings.energia[1] < 50.0, "la víctima pierde energía");
        assert!(w.lemmings.energia[0] > 50.0, "el atacante absorbe");
    }

    #[test]
    fn intercambiar_conserves_total_energy() {
        let mut w = World::new(16, 16);
        w.lemmings.spawn(8.0, 8.0, 30.0, [0.0; 4]);
        w.lemmings.spawn(9.0, 8.0, 30.0, [0.0; 4]);
        let p = SimParams::default();
        w.act_intercambiar(0, &p);
        let total = w.lemmings.energia[0] + w.lemmings.energia[1];
        assert!((total - 60.0).abs() < 1e-4, "la energía se conserva");
    }
}
