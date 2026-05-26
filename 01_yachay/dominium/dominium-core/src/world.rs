//! El mundo: grilla + lemmings, y las 6 acciones atómicas fijas.
//!
//! Cualquier "profesión" o "rol" del macro es sólo un Lemming ejecutando
//! una de estas 6 acciones en un entorno específico.

use crate::conceptos::Conceptos;
use crate::grid::Grid;
use crate::lemmings::{Lemmings, PSI_CORRUPTIBILIDAD, PSI_CURIOSIDAD, PSI_MIEDO, PSI_ORDEN};
use crate::params::{SimParams, TradeTarget};
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
    /// Tick global del mundo — `physics::tick` lo incrementa al final de
    /// cada paso. Es el reloj que alimenta la modulación estacional de
    /// `SimParams::season_period`. Saves viejos sin este campo arrancan
    /// en 0 vía `serde(default)`.
    #[serde(default)]
    pub tick_count: u64,
}

impl World {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            grid: Grid::new(width, height),
            lemmings: Lemmings::new(),
            conceptos: Conceptos::new(),
            tick_count: 0,
        }
    }

    /// Celda que ocupa el Lemming `i`.
    fn cell_of(&self, i: usize) -> usize {
        let (cx, cy) = self.grid.clamp_cell(self.lemmings.pos_x[i], self.lemmings.pos_y[i]);
        self.grid.idx(cx, cy)
    }

    /// Relieve físico de la celda `idx` — combinación lineal de las 5
    /// capas pesada por `p.relieve`. Es la altura que **siente** un
    /// lemming, no la que se renderiza (esa la define `ZWeights`).
    fn relieve_at(&self, idx: usize, p: &SimParams) -> f32 {
        let g = &self.grid;
        p.relieve[0] * g.materia[idx]
            + p.relieve[1] * g.psique[idx]
            + p.relieve[2] * g.poder[idx]
            + p.relieve[3] * g.oro[idx]
            + p.relieve[4] * g.degradacion[idx]
    }

    /// 0 · Mover — gravedad mental hacia el vecino más afín al `vector_psi`,
    /// penalizado por el costo de pendiente. Las "montañas" emergentes de
    /// alta `materia` o de alta `psique` (según `p.relieve`) se vuelven
    /// barreras físicas: cuesta más score subir y se paga energía extra
    /// proporcional a la altura efectivamente subida.
    pub fn act_mover(&mut self, i: usize, p: &SimParams) {
        let (cx, cy) =
            self.grid.clamp_cell(self.lemmings.pos_x[i], self.lemmings.pos_y[i]);
        let psi = self.lemmings.vector_psi[i];
        let cur_idx = self.grid.idx(cx, cy);
        let z_cur = self.relieve_at(cur_idx, p);
        let mut best_dir = (0.0f32, 0.0f32);
        let mut best_z = z_cur;
        let mut best_score = f32::MIN;
        for (dx, dy) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (cx as i64 + dx, cy as i64 + dy);
            if !self.grid.in_bounds(nx, ny) {
                continue;
            }
            let idx = self.grid.idx(nx as usize, ny as usize);
            // Orden busca materia, Miedo evita poder, Curiosidad busca
            // psique, Corruptibilidad busca oro.
            let mut score = psi[PSI_ORDEN] * self.grid.materia[idx]
                - psi[PSI_MIEDO] * self.grid.poder[idx]
                + psi[PSI_CURIOSIDAD] * self.grid.psique[idx]
                + psi[PSI_CORRUPTIBILIDAD] * self.grid.oro[idx];
            let z_n = self.relieve_at(idx, p);
            let climb = (z_n - z_cur).max(0.0);
            score -= p.climb_cost * climb;
            if score > best_score {
                best_score = score;
                best_dir = (dx as f32, dy as f32);
                best_z = z_n;
            }
        }
        let w = self.grid.width as f32 - 1.0;
        let h = self.grid.height as f32 - 1.0;
        self.lemmings.pos_x[i] =
            (self.lemmings.pos_x[i] + best_dir.0 * p.move_speed).clamp(0.0, w);
        self.lemmings.pos_y[i] =
            (self.lemmings.pos_y[i] + best_dir.1 * p.move_speed).clamp(0.0, h);
        // Costo base + costo de pendiente realmente subida.
        let climb_paid = (best_z - z_cur).max(0.0) * p.climb_cost;
        self.lemmings.energia[i] -= p.move_cost + climb_paid;
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

    /// 3 · Intercambiar — transfiere energía a otro agente. El destinatario
    /// depende de `p.trade_target`: `Nearest` mantiene la semántica original
    /// (vecino físico más cercano), `Poorest` redistribuye al más necesitado
    /// del mundo. La elección controla si el sistema alcanza un punto fijo
    /// `N* > 0` o se extingue por desigualdad creciente.
    pub fn act_intercambiar(&mut self, i: usize, p: &SimParams) {
        let target = match p.trade_target {
            TradeTarget::Nearest => self.lemmings.nearest(i),
            TradeTarget::Poorest => self.lemmings.poorest(i),
        };
        let Some(j) = target else { return };
        let amount = p.trade_amount.min(self.lemmings.energia[i]).max(0.0);
        self.lemmings.energia[i] -= amount;
        self.lemmings.energia[j] += amount;
    }

    /// 4 · Replicar — instancia un hijo con edad 0 en una celda **vecina**
    /// (no la misma del padre). El hijo hereda la acción y el `vector_psi`.
    ///
    /// Dispersión determinista: la dirección del hijo viene de
    /// `(edad_padre + idx_padre) % 4`, así N hijos del mismo padre se
    /// reparten en las 4 vecinas. Sin esta dispersión, los hijos saturan
    /// la celda del padre, agotan la materia local y colapsan en cascada
    /// — incluso con regrowth + costo metabólico activos.
    ///
    /// La herencia + dispersión + side-effect de abundancia (ver
    /// `step_lemming`) son las tres piezas que dan al sistema un punto
    /// fijo `N* > 0`.
    pub fn act_replicar(&mut self, i: usize, p: &SimParams) {
        if self.lemmings.energia[i] <= p.replicate_threshold {
            return;
        }
        let cost = self.lemmings.energia[i] * p.child_energy_frac;
        self.lemmings.energia[i] -= cost;
        let psi = self.lemmings.vector_psi[i];
        let accion = self.lemmings.accion[i];
        // Dirección de dispersión: 0=E, 1=O, 2=S, 3=N. Determinista por
        // (edad + i) — distribuye los hijos sucesivos en las 4 vecinas.
        let dir = (self.lemmings.edad[i].wrapping_add(i as u32) & 0x3) as u8;
        let (dx, dy) = match dir {
            0 => (1.0, 0.0),
            1 => (-1.0, 0.0),
            2 => (0.0, 1.0),
            _ => (0.0, -1.0),
        };
        let max_x = self.grid.width as f32 - 1.0;
        let max_y = self.grid.height as f32 - 1.0;
        let x = (self.lemmings.pos_x[i] + dx).clamp(0.0, max_x);
        let y = (self.lemmings.pos_y[i] + dy).clamp(0.0, max_y);
        let child = self.lemmings.spawn(x, y, cost, psi);
        self.lemmings.accion[child] = accion;
    }

    /// 5 · Degradar (Pelear) — resta energía al vecino y absorbe parte.
    pub fn act_degradar(&mut self, i: usize, p: &SimParams) {
        let Some(j) = self.lemmings.nearest(i) else { return };
        let dmg = p.fight_damage.min(self.lemmings.energia[j]).max(0.0);
        self.lemmings.energia[j] -= dmg;
        self.lemmings.energia[i] += dmg * p.absorb_frac;
    }

    /// Despacha la acción del Lemming `i` según su byte `accion`.
    ///
    /// **Bonus de abundancia**: si `p.abundance_threshold > 0` y la
    /// energía del agente supera ese umbral, ejecuta `act_replicar` como
    /// *side-effect* ANTES de su acción principal. Esto cierra el ciclo
    /// termodinámico: cualquier agente saciado se reproduce sin abandonar
    /// su rol (un Extractor sigue extrayendo, un Trader sigue donando).
    /// Si el lemming ya está en `Replicar`, el bonus no doble-cuenta —
    /// `act_replicar` requiere que `energia > replicate_threshold` y le
    /// resta `child_energy_frac`, así que el segundo intento dentro del
    /// mismo tick muy probablemente fallará el guardia.
    pub fn step_lemming(&mut self, i: usize, p: &SimParams) {
        if p.abundance_threshold > 0.0
            && self.lemmings.hack_lock[i] == 0
            && self.lemmings.energia[i] > p.abundance_threshold
        {
            self.act_replicar(i, p);
        }
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
    fn replicar_passes_action_to_child() {
        // Sin herencia, el subgrupo "Replicador" se pierde en una generación
        // y dN/dt < 0 estructuralmente. La herencia es el fix matemático
        // que cierra el ciclo.
        let mut w = World::new(8, 8);
        let i = w.lemmings.spawn(4.0, 4.0, 100.0, [0.5, 0.5, 0.5, 0.5]);
        w.lemmings.accion[i] = 4; // Replicar
        let p = SimParams::default();
        w.act_replicar(i, &p);
        assert_eq!(w.lemmings.len(), 2);
        // El hijo (índice 1) hereda la acción 4 del padre, no la acción 0
        // que el spawn pone por default.
        assert_eq!(w.lemmings.accion[1], 4, "hijo hereda accion del padre");
        // El psi también se hereda — eso ya funcionaba.
        assert_eq!(w.lemmings.vector_psi[1], w.lemmings.vector_psi[0]);
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
    fn mover_prefiere_camino_llano_sobre_subir_pendiente() {
        // Dos vecinos atractivos por materia, uno además requiere subir
        // una montaña (relieve = materia, climb_cost alto). El lemming
        // debe elegir el llano.
        let mut w = World::new(16, 16);
        let i = w.lemmings.spawn(8.0, 8.0, 100.0, [1.0, 0.0, 0.0, 0.0]);
        // Materia idéntica a ambos lados de la celda actual.
        let right = w.grid.idx(9, 8);
        let left = w.grid.idx(7, 8);
        w.grid.materia[right] = 50.0;
        w.grid.materia[left] = 50.0;
        // Pero al subir a la derecha estamos sobre un pico alto.
        // Como `relieve = materia`, el right_idx tiene z=50; el left_idx tiene
        // z=50 también. Para forzar pendiente asimétrica subimos sólo la
        // derecha:
        w.grid.materia[right] = 200.0; // pico mucho mayor
        let mut p = SimParams::default();
        p.climb_cost = 10.0; // pendiente brutalmente cara
        let x0 = w.lemmings.pos_x[i];
        w.act_mover(i, &p);
        // El pico está a la derecha; con climb_cost = 10 cuesta demasiado.
        // Cualquier movimiento que NO sea hacia +x está bien (izq, arriba
        // o abajo son todos llanos).
        assert!(w.lemmings.pos_x[i] <= x0, "no fue hacia el pico de la derecha");
    }

    #[test]
    fn mover_cobra_energia_extra_por_subir() {
        let mut w = World::new(8, 8);
        let i = w.lemmings.spawn(4.0, 4.0, 100.0, [1.0, 0.0, 0.0, 0.0]);
        // Pico a la derecha.
        let right = w.grid.idx(5, 4);
        w.grid.materia[right] = 100.0;
        // Caso A: climb_cost = 0 (sin penalty). Energy gastada = move_cost.
        let mut p = SimParams::default();
        p.climb_cost = 0.0;
        w.act_mover(i, &p);
        let after_flat = w.lemmings.energia[i];
        let lost_flat = 100.0 - after_flat;
        // Reset y repetir con climb_cost > 0.
        let mut w2 = World::new(8, 8);
        let j = w2.lemmings.spawn(4.0, 4.0, 100.0, [1.0, 0.0, 0.0, 0.0]);
        let right2 = w2.grid.idx(5, 4);
        w2.grid.materia[right2] = 100.0;
        let mut p2 = SimParams::default();
        p2.climb_cost = 0.5;
        w2.act_mover(j, &p2);
        let lost_climb = 100.0 - w2.lemmings.energia[j];
        // Sin climb_cost, el agente puede ir igual al pico (porque la
        // materia lo atrae mucho), pero pierde más energía cuando climb_cost
        // > 0 porque paga la altura subida.
        assert!(lost_climb > lost_flat, "subir con climb_cost > 0 cuesta más");
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

    #[test]
    fn intercambiar_poorest_donates_to_the_neediest() {
        // Default: TradeTarget::Poorest. El trader (i=0) tiene E=50.
        // El más cercano (i=1) está al lado pero tiene E=49. El más pobre
        // (i=2) está lejos pero tiene E=5. Debe donar al pobre, no al
        // cercano.
        let mut w = World::new(20, 20);
        w.lemmings.spawn(2.0, 2.0, 50.0, [0.0; 4]); // 0: trader
        w.lemmings.spawn(3.0, 2.0, 49.0, [0.0; 4]); // 1: cercano, rico
        w.lemmings.spawn(18.0, 18.0, 5.0, [0.0; 4]); // 2: lejos, pobre
        let p = SimParams::default();
        let before_close = w.lemmings.energia[1];
        let before_poor = w.lemmings.energia[2];
        w.act_intercambiar(0, &p);
        assert_eq!(w.lemmings.energia[1], before_close, "no le tocó al cercano");
        assert!(w.lemmings.energia[2] > before_poor, "le donó al pobre");
    }

    #[test]
    fn intercambiar_nearest_preserves_legacy_behavior() {
        // Con TradeTarget::Nearest, el comportamiento histórico se mantiene.
        let mut w = World::new(20, 20);
        w.lemmings.spawn(2.0, 2.0, 50.0, [0.0; 4]);
        w.lemmings.spawn(3.0, 2.0, 49.0, [0.0; 4]); // cercano
        w.lemmings.spawn(18.0, 18.0, 5.0, [0.0; 4]); // pobre lejos
        let mut p = SimParams::default();
        p.trade_target = TradeTarget::Nearest;
        let before_close = w.lemmings.energia[1];
        let before_poor = w.lemmings.energia[2];
        w.act_intercambiar(0, &p);
        assert!(w.lemmings.energia[1] > before_close, "le donó al cercano");
        assert_eq!(w.lemmings.energia[2], before_poor, "no le tocó al pobre");
    }
}
