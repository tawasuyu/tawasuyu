//! `Critter` — un agente que **deambula** el mundo voxel por su cuenta: la
//! misma física de cuerpo que el jugador ([`Player`]) más una voluntad mínima
//! (cambia de rumbo cada tanto, salta a veces, gira al chocar). Sirve para
//! poblar un mundo voxel de bichos vivos; el render los dibuja como cajas
//! analíticas en el mismo pase de ray-march ([`llimphi_3d::Entity3d`]).
//!
//! Determinista (sin `rand`): cada bicho lleva su propia semilla y avanza un
//! LCG, así un mundo se reproduce igual (y los tests/PNG son estables).

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Entity3d, VoxelGrid};

use crate::{forward_h, Player};

const TAU: f32 = std::f32::consts::TAU;
/// Velocidad de pasto de un bicho (más lento que el jugador).
const CRITTER_SPEED: f32 = 3.2;
/// Semi-tamaño de la caja del bicho (ancho, alto, fondo) en voxels.
const HALF: [f32; 3] = [0.5, 0.7, 0.5];

/// Bicho que deambula: un [`Player`] (física) + rumbo y temporizadores de IA.
#[derive(Debug, Clone, Copy)]
pub struct Critter {
    /// Cuerpo físico (gravedad + colisión), reusado del jugador.
    pub body: Player,
    /// Color de la caja al dibujarlo.
    pub color: [u8; 3],
    /// Rumbo actual de caminata (yaw, rad).
    heading: f32,
    /// Segundos hasta el próximo cambio de rumbo.
    timer: f32,
    /// Pide saltar en el próximo paso (one-shot).
    jump: bool,
    /// Estado del LCG (azar determinista por bicho).
    rng: u32,
}

impl Critter {
    /// Bicho posado sobre la columna `(x, z)` del grid, con `color` y `seed`
    /// (distintas semillas → rumbos distintos).
    pub fn spawn_on(grid: &VoxelGrid, x: u32, z: u32, color: [u8; 3], seed: u32) -> Self {
        let mut body = Player::spawn_on(grid, x, z);
        body.speed = CRITTER_SPEED;
        let mut c = Self {
            body,
            color,
            heading: 0.0,
            timer: 0.0,
            jump: false,
            rng: seed | 1, // nunca 0 (un LCG con 0 se queda pegado en patrones pobres)
        };
        c.pick_new_goal();
        c
    }

    /// Avanza un `dt`: actualiza la voluntad y mueve el cuerpo. Llamar por frame.
    pub fn step(&mut self, grid: &VoxelGrid, dt: f32) {
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.pick_new_goal();
        }

        let before = self.body.pos;
        let wish = forward_h(self.heading);
        let jump = self.jump && self.body.on_ground;
        self.body.step(grid, wish, jump, dt);
        self.jump = false;

        // ¿Chocó? (apenas se movió en horizontal estando en el piso) → media
        // vuelta y nuevo rumbo, así no se queda empujando una pared.
        let dx = self.body.pos.x - before.x;
        let dz = self.body.pos.z - before.z;
        let moved = (dx * dx + dz * dz).sqrt();
        if self.body.on_ground && moved < 0.2 * CRITTER_SPEED * dt {
            self.heading += TAU * 0.5 + (self.next_f32() - 0.5);
            self.timer = 0.6 + self.next_f32();
        }
    }

    /// Caja analítica para el renderer: centro = pies + medio-alto, así la caja
    /// se apoya en el suelo (los pies del cuerpo).
    pub fn entity(&self) -> Entity3d {
        Entity3d {
            pos: [self.body.pos.x, self.body.pos.y + HALF[1], self.body.pos.z],
            half: HALF,
            color: self.color,
        }
    }

    /// Elige rumbo, duración y un eventual salto nuevos.
    fn pick_new_goal(&mut self) {
        self.heading = self.next_f32() * TAU;
        self.timer = 0.8 + self.next_f32() * 2.5;
        self.jump = self.next_f32() < 0.18;
    }

    /// Próximo `f32` en `[0, 1)` del LCG (Numerical Recipes).
    fn next_f32(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        // Bits altos (más aleatorios) → mantisa de 24 bits.
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Grid 32³ con piso sólido en `y=0`.
    fn grid_con_piso() -> VoxelGrid {
        let mut g = VoxelGrid::new([32, 8, 32]);
        for z in 0..32 {
            for x in 0..32 {
                g.set(x, 0, z, [90, 140, 80]);
            }
        }
        g
    }

    #[test]
    fn deambula_y_no_atraviesa_el_piso() {
        let g = grid_con_piso();
        let start = Vec3::new(16.5, 1.0, 16.5);
        let mut c = Critter::spawn_on(&g, 16, 16, [220, 220, 220], 7);
        assert!((c.body.pos - start).length() < 0.1);
        for _ in 0..600 {
            c.step(&g, 1.0 / 60.0);
            // Nunca cae por debajo del piso (pies en y≈1).
            assert!(c.body.pos.y >= 1.0 - 0.05, "se hundió: y={}", c.body.pos.y);
        }
        // Tras 10 s deambulando, se movió de donde arrancó.
        let dx = c.body.pos.x - start.x;
        let dz = c.body.pos.z - start.z;
        assert!((dx * dx + dz * dz).sqrt() > 1.0, "no deambuló");
    }

    #[test]
    fn rebota_y_queda_dentro_del_corral() {
        // Corral 8×8 con paredes altas; el bicho debe quedar adentro.
        let mut g = VoxelGrid::new([8, 6, 8]);
        for z in 0..8 {
            for x in 0..8 {
                g.set(x, 0, z, [90, 140, 80]);
                if x == 0 || x == 7 || z == 0 || z == 7 {
                    for y in 1..6 {
                        g.set(x, y, z, [120, 120, 120]);
                    }
                }
            }
        }
        let mut c = Critter::spawn_on(&g, 4, 4, [220, 180, 120], 3);
        for _ in 0..1200 {
            c.step(&g, 1.0 / 60.0);
        }
        // Dentro del corral interior (1..7) con su medio-ancho.
        assert!(c.body.pos.x > 1.0 - HALF[0] && c.body.pos.x < 7.0 + HALF[0], "x fuera: {}", c.body.pos.x);
        assert!(c.body.pos.z > 1.0 - HALF[0] && c.body.pos.z < 7.0 + HALF[0], "z fuera: {}", c.body.pos.z);
    }

    #[test]
    fn dos_semillas_distintas_divergen() {
        let g = grid_con_piso();
        let mut a = Critter::spawn_on(&g, 16, 16, [255; 3], 1);
        let mut b = Critter::spawn_on(&g, 16, 16, [255; 3], 999);
        for _ in 0..300 {
            a.step(&g, 1.0 / 60.0);
            b.step(&g, 1.0 / 60.0);
        }
        assert!((a.body.pos - b.body.pos).length() > 0.5, "semillas no divergen");
    }
}
