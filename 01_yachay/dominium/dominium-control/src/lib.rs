//! `dominium-control` — estabilidad por **lazo cerrado**.
//!
//! La física de dominium ya tiene mucha maquinaria de equilibrio (regrowth
//! logístico, costo metabólico, densidad-dependencia, saturación de campos,
//! atractores de abundancia/desesperación). Pero es **lazo abierto**: fijás las
//! palancas a mano y rezás para que `N*` caiga donde querés. Los comentarios de
//! `params.rs` lo confesaban: *"si subís estos, N\* explota — validá
//! empíricamente"*.
//!
//! Este crate cierra el lazo. Le decís el **setpoint** de población y el
//! controlador mide `N(t)` y mueve **una palanca blanda** (regrowth /
//! metabolismo / capacidad de carga) para sostenerlo — sin tocar la base
//! inamovible del §1: no agrega fases al tick, sólo reescribe escalares de
//! `SimParams` *entre* ticks, exactamente lo que un humano haría con un slider,
//! pero en automático y converso.
//!
//! **Determinista**: el PID es `+`/`−`/`×` de `f32` en orden fijo (sin RNG, sin
//! transcendentales) → bit-exacto plataforma a plataforma, igual que el motor.
//! Se certifica corriendo la **sim real** (`dominium-physics::tick`) y midiendo
//! convergencia, no con asserts de juguete.

use dominium_core::{Grid, SimParams, World};
use dominium_physics::tick;

/// Qué escalar de `SimParams` mueve el controlador para regular la población.
/// Son las palancas "blandas": tocan el flujo termodinámico (la fuente o el
/// sumidero de energía) sin alterar la ontología del agente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lever {
    /// `regrowth_rate` — fuente de materia → energía → natalidad. Subirla hace
    /// crecer `N`. Es la palanca más directa y la default.
    Regrowth,
    /// `carrying_capacity` — asíntota del regrowth. Más capacidad ⇒ más materia
    /// disponible ⇒ más `N`.
    CarryingCapacity,
    /// `metabolic_cost` — sumidero basal de energía. Subirlo **mata** más ⇒
    /// baja `N` (efecto **inverso**: el controlador lo sabe y corrige el signo).
    Metabolic,
}

impl Lever {
    /// Signo del efecto de la palanca sobre `N`: `+1` si subirla sube la
    /// población, `−1` si la baja. El controlador lo usa para empujar en la
    /// dirección correcta sin que el usuario tenga que pensarlo.
    fn effect_sign(self) -> f32 {
        match self {
            Lever::Regrowth | Lever::CarryingCapacity => 1.0,
            Lever::Metabolic => -1.0,
        }
    }

    /// Lee el valor actual de la palanca en `params`.
    fn read(self, p: &SimParams) -> f32 {
        match self {
            Lever::Regrowth => p.regrowth_rate,
            Lever::CarryingCapacity => p.carrying_capacity,
            Lever::Metabolic => p.metabolic_cost,
        }
    }

    /// Escribe el valor de la palanca en `params`.
    fn write(self, p: &mut SimParams, v: f32) {
        match self {
            Lever::Regrowth => p.regrowth_rate = v,
            Lever::CarryingCapacity => p.carrying_capacity = v,
            Lever::Metabolic => p.metabolic_cost = v,
        }
    }

    /// Rango por defecto `(lo, hi)` razonable para la palanca, si el caller no
    /// fija uno propio. Acotan el espacio de control y dan anti-windup natural
    /// (la palanca satura, el integrador no se desboca).
    fn default_bounds(self) -> (f32, f32) {
        match self {
            Lever::Regrowth => (0.0, 0.25),
            Lever::CarryingCapacity => (1.0, 120.0),
            Lever::Metabolic => (0.0, 0.6),
        }
    }
}

/// Controlador homeostático PID en **forma de velocidad** (incremental). Mide la
/// población cada `period` ticks y ajusta la palanca hacia el `setpoint`.
///
/// La forma de velocidad (acumula Δ sobre la palanca, no recalcula un absoluto)
/// tiene anti-windup incorporado: la palanca se clampa a `bounds` y el estado
/// interno sólo guarda dos errores previos, así que en régimen estable
/// (`e ≈ 0`) la palanca **se queda quieta** en el valor que halló — que es,
/// justamente, la calibración que un humano buscaría a tientas.
#[derive(Debug, Clone)]
pub struct StabilityController {
    /// Población objetivo (cantidad de lemmings vivos).
    pub setpoint: f32,
    /// Qué palanca mueve.
    pub lever: Lever,
    /// Ganancia proporcional (sobre el cambio de error normalizado).
    pub kp: f32,
    /// Ganancia integral (sobre el error normalizado).
    pub ki: f32,
    /// Ganancia derivativa (sobre la curvatura del error normalizado).
    pub kd: f32,
    /// Cada cuántos ticks actúa. Períodos chicos persiguen ruido; grandes,
    /// inerte. `~15` anda bien para grillas de decenas de miles de celdas.
    pub period: u32,
    /// Cota `(lo, hi)` de la palanca.
    pub bounds: (f32, f32),

    /// Variable de control interna ∈ `[0, 1]` (fracción del rango `bounds`).
    /// La palanca real es `lo + c · (hi − lo)`.
    c: f32,
    /// Errores normalizados previos (forma de velocidad).
    e_prev: f32,
    e_prev2: f32,
    /// Cuántas veces ya actuó (para no usar `e_prev`/`e_prev2` antes de tiempo).
    steps: u32,
    /// Último tick en que actuó.
    last_tick: u64,
    /// Si ya inicializó `c` desde el valor inicial de la palanca.
    primed: bool,
}

impl StabilityController {
    /// Controlador con ganancias por defecto sintonizadas para grillas medianas
    /// (~50²–240²). Arranca la palanca desde el valor que ya tenga `params` la
    /// primera vez que actúa.
    pub fn new(setpoint: f32, lever: Lever) -> Self {
        Self {
            setpoint: setpoint.max(0.0),
            lever,
            // Sintonía en espacio de fracción-de-rango. La planta tiene lag
            // grande (cambiar la fuente tarda en propagar materia→energía→
            // natalidad), así que el integrador lleva el peso (mata el error de
            // régimen) y el proporcional/derivativo amortiguan el transitorio.
            kp: 0.22,
            ki: 0.13,
            kd: 0.05,
            period: 20,
            bounds: lever.default_bounds(),
            c: 0.0,
            e_prev: 0.0,
            e_prev2: 0.0,
            steps: 0,
            last_tick: 0,
            primed: false,
        }
    }

    /// Fija un rango de palanca propio (sobreescribe el default).
    pub fn with_bounds(mut self, lo: f32, hi: f32) -> Self {
        self.bounds = (lo.min(hi), lo.max(hi));
        self
    }

    /// Fija ganancias y período propios.
    pub fn with_gains(mut self, kp: f32, ki: f32, kd: f32, period: u32) -> Self {
        self.kp = kp;
        self.ki = ki;
        self.kd = kd;
        self.period = period.max(1);
        self
    }

    /// Valor actual de la palanca (en sus unidades reales).
    pub fn lever_value(&self) -> f32 {
        let (lo, hi) = self.bounds;
        lo + self.c * (hi - lo)
    }

    /// Considera actuar en `tick`. Si pasó `period` desde la última acción, mide
    /// `N`, corre el PID y reescribe la palanca en `params`. Devuelve `true` si
    /// actuó este tick. Llamalo **una vez por tick**, *después* de `tick()`.
    pub fn observe(&mut self, world: &World, params: &mut SimParams, tick_now: u64) -> bool {
        // Primer contacto: sembrar `c` desde el valor que el caller ya puso en
        // la palanca, así el lazo arranca de la calibración existente y no de un
        // salto brusco.
        if !self.primed {
            let (lo, hi) = self.bounds;
            let span = (hi - lo).max(f32::EPSILON);
            self.c = ((self.lever.read(params) - lo) / span).clamp(0.0, 1.0);
            self.primed = true;
            self.last_tick = tick_now;
            return false;
        }
        if tick_now.saturating_sub(self.last_tick) < self.period as u64 {
            return false;
        }
        self.last_tick = tick_now;

        let n = world.lemmings.len() as f32;
        // Error normalizado por el setpoint → ganancias independientes de la
        // escala de población (anda igual con setpoint 200 o 20.000).
        let sp = self.setpoint.max(1.0);
        let e = (self.setpoint - n) / sp;

        // PID de velocidad: Δc = kp·(e−e₋₁) + ki·e + kd·(e − 2e₋₁ + e₋₂).
        let (de, dde) = if self.steps == 0 {
            (0.0, 0.0)
        } else if self.steps == 1 {
            (e - self.e_prev, 0.0)
        } else {
            (e - self.e_prev, e - 2.0 * self.e_prev + self.e_prev2)
        };
        let dc = self.kp * de + self.ki * e + self.kd * dde;

        // El signo de la palanca corrige la dirección (metabolismo es inverso).
        self.c = (self.c + self.lever.effect_sign() * dc).clamp(0.0, 1.0);
        self.lever.write(params, self.lever_value());

        self.e_prev2 = self.e_prev;
        self.e_prev = e;
        self.steps += 1;
        true
    }
}

/// **Techo ecológico** analítico: el máximo de población que la
/// densidad-dependencia permite, leído directo de los parámetros (no estimado —
/// es exacto para ese mecanismo). Cada bloque `density_block²` admite a lo sumo
/// `density_cap` lemmings que se repliquen, así que el mundo satura en
/// `bloques · density_cap`. `None` si la densidad-dependencia está apagada
/// (`density_block == 0` o `density_cap == 0`) — entonces no hay techo ecológico
/// y sólo frena el tope duro `max_population`.
///
/// Sirve como **predictor** para el panel ("con estos factores, N no pasará de
/// X") y como cota superior sana para el `setpoint` del controlador.
pub fn density_ceiling(grid: &Grid, params: &SimParams) -> Option<u32> {
    if params.density_block == 0 || params.density_cap == 0 {
        return None;
    }
    let b = params.density_block as usize;
    let blocks_x = grid.width.div_ceil(b);
    let blocks_y = grid.height.div_ceil(b);
    Some((blocks_x * blocks_y) as u32 * params.density_cap)
}

/// Corre la sim `ticks` pasos con el controlador en el lazo y devuelve la
/// **trayectoria de población** `N(t)` (un valor por tick) más el valor final de
/// la palanca. Re-siembra con `reseed` si la población colapsa a cero, para que
/// el controlador tenga a quién regular (un mundo vacío no se puede estabilizar
/// desde adentro). Helper para tests, CLI y el panel.
pub fn run_controlled(
    world: &mut World,
    params: &mut SimParams,
    ctrl: &mut StabilityController,
    ticks: u32,
    mut reseed: impl FnMut() -> World,
) -> (Vec<u32>, f32) {
    let mut traj = Vec::with_capacity(ticks as usize);
    for t in 0..ticks as u64 {
        tick(world, params);
        if world.lemmings.is_empty() {
            *world = reseed();
        }
        ctrl.observe(world, params, t);
        traj.push(world.lemmings.len() as u32);
    }
    (traj, ctrl.lever_value())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dominium_core::{worldgen, Conceptos};

    /// Mundo sembrado reproducible para los tests de convergencia.
    fn seed_world(seed: u64, grid: usize, pop: usize) -> World {
        worldgen::seed(seed, grid, pop, Conceptos::default())
    }

    /// Parámetros base con los frenos defensivos que la app enciende (para que
    /// un setpoint alto no cuelgue el O(N²) ni diverjan los campos).
    fn base_params() -> SimParams {
        let mut p = SimParams::default();
        p.field_saturation = 150.0;
        p.max_energy = 400.0;
        p
    }

    /// Métrica de convergencia: error relativo medio de `N` al setpoint en el
    /// último tercio de la corrida (régimen, no transitorio).
    fn tail_rel_error(traj: &[u32], setpoint: f32) -> f32 {
        let start = traj.len() * 2 / 3;
        let tail = &traj[start..];
        let mean: f32 =
            tail.iter().map(|&n| (n as f32 - setpoint).abs() / setpoint).sum::<f32>()
                / tail.len() as f32;
        mean
    }

    #[test]
    fn techo_ecologico_es_exacto() {
        let g = Grid::new(100, 80);
        let mut p = SimParams::default();
        p.density_block = 10;
        p.density_cap = 5;
        // 10 bloques en x (100/10), 8 en y (80/10) → 80 bloques × 5 = 400.
        assert_eq!(density_ceiling(&g, &p), Some(400));
        // Apagada → sin techo ecológico.
        p.density_cap = 0;
        assert_eq!(density_ceiling(&g, &p), None);
    }

    #[test]
    fn frena_una_poblacion_que_explotaria() {
        // Arranque caliente: regrowth alto y SIN densidad-dependencia → en lazo
        // abierto la población dispara. El controlador (palanca = regrowth) debe
        // domarla al setpoint.
        let grid = 64;
        let setpoint = 500.0;
        let mut w = seed_world(0x00E5_0D1E_u64, grid, grid * 5);
        let mut p = base_params();
        p.regrowth_rate = 0.22; // fuente desbordada
        let mut ctrl = StabilityController::new(setpoint, Lever::Regrowth);
        let (traj, lever) =
            run_controlled(&mut w, &mut p, &mut ctrl, 900, || seed_world(1, grid, grid * 5));
        let err = tail_rel_error(&traj, setpoint);
        let last = *traj.last().unwrap();
        eprintln!(
            "explode→domado: N_final={last} lever_regrowth={lever:.4} err_cola={:.1}%",
            err * 100.0
        );
        assert!(err < 0.25, "el controlador sostiene N cerca del setpoint (err {err:.2})");
        assert!(last > 0, "no se extinguió");
    }

    #[test]
    fn rescata_una_poblacion_que_colapsaria() {
        // Arranque frío: regrowth casi nulo → en lazo abierto la población se
        // extingue. El controlador debe SUBIR el regrowth y levantar N al
        // setpoint en vez de dejarla morir.
        let grid = 64;
        let setpoint = 400.0;
        let mut w = seed_world(0xC07D_u64, grid, grid * 5);
        let mut p = base_params();
        p.regrowth_rate = 0.0; // fuente apagada
        let mut ctrl = StabilityController::new(setpoint, Lever::Regrowth);
        let (traj, lever) =
            run_controlled(&mut w, &mut p, &mut ctrl, 900, || seed_world(2, grid, grid * 5));
        let err = tail_rel_error(&traj, setpoint);
        let last = *traj.last().unwrap();
        eprintln!(
            "collapse→rescatado: N_final={last} lever_regrowth={lever:.4} err_cola={:.1}%",
            err * 100.0
        );
        assert!(lever > 0.0, "el controlador encendió la fuente de materia");
        assert!(err < 0.30, "N converge al setpoint pese al arranque frío (err {err:.2})");
        assert!(last > 50, "rescató la población de la extinción");
    }

    #[test]
    fn determinista_bit_exacto() {
        // Dos corridas idénticas → trayectoria idéntica (sin RNG ni floats
        // divergentes en el lazo).
        let grid = 48;
        let run = || {
            let mut w = seed_world(7, grid, grid * 4);
            let mut p = base_params();
            p.regrowth_rate = 0.10;
            let mut c = StabilityController::new(300.0, Lever::Regrowth);
            run_controlled(&mut w, &mut p, &mut c, 300, || seed_world(7, grid, grid * 4)).0
        };
        assert_eq!(run(), run(), "el lazo es determinista bit-exacto");
    }
}
