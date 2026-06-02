//! Sesión de simulación de dominium — el estado de dominio y su ciclo de
//! vida, separados del frontend (regla #2).
//!
//! [`Sim`] posee el `World`, sus `SimParams`, el reloj (`tick`/`epoch`/
//! `rng_seed`), el ring de snapshots para rebobinar, los trails de posiciones
//! y las asignaciones de cluster k-means. Orquesta el avance del motor
//! (`dominium-physics`), el reseed manual y el reseed automático cuando la
//! población colapsa. El frontend (cámara, selección, render, paneles, menús)
//! sólo guarda estado de vista y delega el ciclo de vida acá — antes todo
//! esto vivía mezclado en el `Model` de `dominium-app-llimphi`.

use std::collections::VecDeque;

use dominium_core::{kmeans_psi, SimParams, World};
use dominium_physics::tick;

/// Cómo re-sembrar el mundo a partir de una semilla. El frontend la provee
/// (típicamente cargando su pack de Conceptos del disco), así `Sim` no
/// conoce ni el tamaño de grilla ni la IO de packs.
pub type Seeder = Box<dyn FnMut(u64) -> World>;

/// Sesión de simulación: estado de dominio + reloj + historia.
pub struct Sim {
    pub world: World,
    pub params: SimParams,
    /// Si el motor avanza en cada [`Sim::step`]. El frontend lo togglea.
    pub running: bool,
    pub tick: u64,
    pub epoch: u64,
    pub rng_seed: u64,
    /// Ring de snapshots del `World` — el último es el más reciente. Ver
    /// [`Sim::displayed_world`] para la semántica de `rewind_offset`.
    pub snapshots: VecDeque<World>,
    /// Cuántos pasos atrás mira el usuario. `0` = presente (vivo).
    pub rewind_offset: usize,
    /// Para cada frame reciente, las posiciones `(x, y)` de los lemmings
    /// vivos. `trails[k]` es el frame `tick - (len-1-k)`.
    pub trails: VecDeque<Vec<(f32, f32)>>,
    /// Asignación k-means → cluster por lemming (modo PsiCluster).
    pub cluster_assignments: Vec<u8>,
    /// Tick del último refresh de clusters (gated refresh).
    pub cluster_last_refresh: u64,

    snapshot_cap: usize,
    trail_cap: usize,
    kmeans_refresh_ticks: u64,
    seeder: Seeder,
}

impl Sim {
    /// Arranca una sesión con un `World` ya sembrado, sus parámetros, la
    /// semilla del PRNG de reseed, las capacidades de los rings, el periodo
    /// de refresh de k-means y el `seeder` que produce mundos nuevos.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        world: World,
        params: SimParams,
        rng_seed: u64,
        snapshot_cap: usize,
        trail_cap: usize,
        kmeans_refresh_ticks: u64,
        running: bool,
        seeder: Seeder,
    ) -> Self {
        Self {
            world,
            params,
            running,
            tick: 0,
            epoch: 0,
            rng_seed,
            snapshots: VecDeque::with_capacity(snapshot_cap),
            rewind_offset: 0,
            trails: VecDeque::with_capacity(trail_cap),
            cluster_assignments: Vec::new(),
            cluster_last_refresh: 0,
            snapshot_cap,
            trail_cap,
            kmeans_refresh_ticks,
            seeder,
        }
    }

    /// Un paso de simulación; re-siembra si la población colapsa. Captura
    /// también el snapshot del estado y el frame de trails (después de
    /// avanzar, así el "presente" siempre coincide con `world`). Recalcula
    /// los clusters sólo si `needs_clusters` y pasó el periodo de refresh —
    /// el frontend pasa `true` cuando el render lo necesita (modo PsiCluster).
    pub fn advance(&mut self, needs_clusters: bool) {
        tick(&mut self.world, &self.params);
        self.tick += 1;
        if self.world.lemmings.is_empty() {
            self.epoch += 1;
            self.rng_seed = self.rng_seed.wrapping_mul(2862933555777941757).wrapping_add(1);
            self.world = (self.seeder)(self.rng_seed);
            self.tick = 0;
            self.snapshots.clear();
            self.trails.clear();
            self.cluster_assignments.clear();
        }
        self.push_snapshot();
        self.push_trail_frame();
        if needs_clusters
            && self.tick.saturating_sub(self.cluster_last_refresh) >= self.kmeans_refresh_ticks
        {
            self.refresh_clusters();
        }
    }

    /// Reseed manual: nueva semilla, mundo fresco, reloj a cero y rings
    /// limpios. Vuelve al presente (`rewind_offset = 0`).
    pub fn reseed(&mut self) {
        self.rng_seed = self.rng_seed.wrapping_add(0x9E37_79B9);
        self.world = (self.seeder)(self.rng_seed);
        self.tick = 0;
        self.epoch += 1;
        self.snapshots.clear();
        self.trails.clear();
        self.rewind_offset = 0;
    }

    /// Recalcula `cluster_assignments` desde el `World` actual. Si
    /// `kmeans_psi` devuelve `None` (pob < K), limpia las asignaciones.
    pub fn refresh_clusters(&mut self) {
        if let Some(km) = kmeans_psi(&self.world) {
            self.cluster_assignments = km.assignments;
        } else {
            self.cluster_assignments.clear();
        }
        self.cluster_last_refresh = self.tick;
    }

    /// Empuja el `World` actual al ring (clone barato: SoA + Vec). Drop del
    /// más viejo al exceder la capacidad.
    pub fn push_snapshot(&mut self) {
        if self.snapshots.len() == self.snapshot_cap {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(self.world.clone());
    }

    /// Empuja el frame de posiciones de todos los lemmings vivos al ring.
    pub fn push_trail_frame(&mut self) {
        if self.trails.len() == self.trail_cap {
            self.trails.pop_front();
        }
        let lem = &self.world.lemmings;
        let frame: Vec<(f32, f32)> = (0..lem.len()).map(|i| (lem.pos_x[i], lem.pos_y[i])).collect();
        self.trails.push_back(frame);
    }

    /// Devuelve el `World` que actualmente se está mostrando — el presente
    /// (`world`) si no hay rewind, o el snapshot apropiado si lo hay.
    pub fn displayed_world(&self) -> &World {
        if self.rewind_offset == 0 || self.snapshots.is_empty() {
            &self.world
        } else {
            let len = self.snapshots.len();
            let idx = len.saturating_sub(1 + self.rewind_offset);
            &self.snapshots[idx]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dominium_core::World;

    // Seeder de prueba: un mundo chico determinista vacío de lemmings vivos
    // sólo cuando lo pidamos. Para la mayoría de los tests no colapsa.
    fn poblar(w: &mut World) {
        for k in 0..8 {
            w.lemmings.spawn(k as f32 % 4.0, 0.0, 50.0, [0.5; 4]);
        }
    }

    fn sim_demo() -> Sim {
        // Mundo mínimo 4×4 con algunos lemmings para que no colapse al toque.
        let mut w = World::new(4, 4);
        poblar(&mut w);
        let seeder: Seeder = Box::new(|_s| {
            let mut w = World::new(4, 4);
            poblar(&mut w);
            w
        });
        Sim::new(w, SimParams::default(), 0xC0FFEE, 4, 3, 30, true, seeder)
    }

    #[test]
    fn advance_incrementa_tick_y_captura_historia() {
        let mut s = sim_demo();
        s.advance(false);
        assert_eq!(s.tick, 1);
        assert_eq!(s.snapshots.len(), 1);
        assert_eq!(s.trails.len(), 1);
    }

    #[test]
    fn rings_respetan_su_capacidad() {
        let mut s = sim_demo();
        for _ in 0..10 {
            s.advance(false);
        }
        assert!(s.snapshots.len() <= 4, "snapshot ring capado");
        assert!(s.trails.len() <= 3, "trail ring capado");
    }

    #[test]
    fn reseed_resetea_reloj_y_vuelve_al_presente() {
        let mut s = sim_demo();
        s.advance(false);
        s.advance(false);
        s.rewind_offset = 1;
        s.reseed();
        assert_eq!(s.tick, 0);
        assert_eq!(s.rewind_offset, 0);
        assert!(s.snapshots.is_empty());
        assert_eq!(s.epoch, 1);
    }

    #[test]
    fn displayed_world_sigue_el_rewind() {
        let mut s = sim_demo();
        s.advance(false);
        s.advance(false);
        // presente
        s.rewind_offset = 0;
        assert!(std::ptr::eq(s.displayed_world(), &s.world));
        // pasado: devuelve un snapshot, no el world vivo
        s.rewind_offset = 1;
        assert!(!std::ptr::eq(s.displayed_world(), &s.world));
    }
}
