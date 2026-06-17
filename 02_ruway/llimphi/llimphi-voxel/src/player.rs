//! Física de jugador en primera persona sobre un [`VoxelGrid`] — el otro
//! ingrediente núcleo de un juego voxel (además del picking de [`raycast`]):
//! **caminar el mundo con gravedad y colisión**. Reusable por cualquier juego
//! voxel de la suite; no toca la GPU ni el render.
//!
//! Coordenadas en **espacio de grilla** (voxel = 1, mundo en `[0, dim]`), las
//! mismas que [`raycast`](crate::raycast) y `eye_mundo + dim/2`. El jugador es
//! una caja AABB parada sobre el terreno; `pos` es el **centro de los pies**
//! (x/z centrados, y en la base). La colisión se resuelve eje por eje
//! (move-and-resolve), el método clásico y robusto para voxels.
//!
//! La mirada (`yaw`/`pitch`) la lleva la app (compartida con la cámara
//! `orbit`/`fly`); acá viven sólo las helpers puras de base
//! ([`forward_h`]/[`right_h`]/[`look_dir`]) para derivar el vector de avance.

use llimphi_3d::glam::Vec3;
use llimphi_3d::VoxelGrid;

/// Semi-ancho del jugador en X/Z (la caja mide `2·HALF_W` de lado).
const HALF_W: f32 = 0.3;
/// Altura total de la caja (de los pies a la coronilla).
const HEIGHT: f32 = 1.7;
/// Altura del ojo sobre los pies (la cámara va acá).
const EYE: f32 = 1.55;
/// Aceleración de gravedad (voxels/s²).
const GRAVITY: f32 = 30.0;
/// Velocidad horizontal de caminata (voxels/s).
const MOVE_SPEED: f32 = 8.0;
/// Velocidad vertical inicial del salto (voxels/s).
const JUMP_SPEED: f32 = 9.0;
/// Margen para no “tocar” el voxel del plano siguiente por error de redondeo.
const EPS: f32 = 1e-3;

/// Avance horizontal unitario para `yaw` (en grilla): `yaw=0` mira a `+Z`.
pub fn forward_h(yaw: f32) -> Vec3 {
    let (s, c) = yaw.sin_cos();
    Vec3::new(s, 0.0, c)
}

/// "Derecha" horizontal unitaria para `yaw` (perpendicular a [`forward_h`],
/// con la convención de mano derecha del motor: a `yaw=0`, derecha = `+X`).
pub fn right_h(yaw: f32) -> Vec3 {
    let (s, c) = yaw.sin_cos();
    Vec3::new(c, 0.0, -s)
}

/// Dirección de mirada completa (incluye `pitch`), misma convención que
/// [`Camera3d::fly`](llimphi_3d::Camera3d::fly). Útil como `dir` del raycast.
pub fn look_dir(yaw: f32, pitch: f32) -> Vec3 {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    Vec3::new(cp * sy, sp, cp * cy)
}

/// Jugador físico: una caja AABB con velocidad, parada sobre el terreno.
#[derive(Debug, Clone, Copy)]
pub struct Player {
    /// Centro de los pies (espacio de grilla).
    pub pos: Vec3,
    /// Velocidad actual (voxels/s).
    pub vel: Vec3,
    /// `true` si el último paso terminó apoyado en suelo (habilita el salto).
    pub on_ground: bool,
}

impl Player {
    /// Jugador parado con los pies en `feet` (centro de los pies, grilla).
    pub fn new(feet: Vec3) -> Self {
        Self {
            pos: feet,
            vel: Vec3::ZERO,
            on_ground: false,
        }
    }

    /// Crea un jugador posado sobre la columna `(x, z)` del grid: pies un voxel
    /// por encima del suelo más alto (o sobre el piso `y=0` si la columna está
    /// vacía). Centrado en el voxel (`+0.5`).
    pub fn spawn_on(grid: &VoxelGrid, x: u32, z: u32) -> Self {
        let top = grid.height_at(x, z).map(|y| y as f32 + 1.0).unwrap_or(0.0);
        Self::new(Vec3::new(x as f32 + 0.5, top + EPS, z as f32 + 0.5))
    }

    /// Posición del ojo (cámara) en espacio de grilla.
    pub fn eye(&self) -> Vec3 {
        self.pos + Vec3::new(0.0, EYE, 0.0)
    }

    /// Avanza la física un paso `dt` segundos. `wish` es la dirección de
    /// caminata **horizontal** deseada (no necesita estar normalizada; se usa
    /// su dirección × [`MOVE_SPEED`]); `jump` solicita un salto (sólo prende si
    /// `on_ground`). Resuelve colisión contra `grid` eje por eje.
    pub fn step(&mut self, grid: &VoxelGrid, wish: Vec3, jump: bool, dt: f32) {
        let dt = dt.clamp(0.0, 0.05); // nunca un salto de física monstruoso

        // Velocidad horizontal: directa desde el deseo (sin inercia, simple).
        let flat = Vec3::new(wish.x, 0.0, wish.z);
        let h = if flat.length_squared() > 1e-6 {
            flat.normalize() * MOVE_SPEED
        } else {
            Vec3::ZERO
        };
        self.vel.x = h.x;
        self.vel.z = h.z;

        // Gravedad + salto.
        self.vel.y -= GRAVITY * dt;
        if jump && self.on_ground {
            self.vel.y = JUMP_SPEED;
        }

        // Mover y resolver eje por eje. on_ground se re-evalúa cada paso.
        self.on_ground = false;
        self.move_axis(grid, 0, self.vel.x * dt);
        self.move_axis(grid, 1, self.vel.y * dt);
        self.move_axis(grid, 2, self.vel.z * dt);
    }

    /// AABB `[min, max]` del jugador en `pos`.
    fn aabb_at(&self, pos: Vec3) -> (Vec3, Vec3) {
        (
            Vec3::new(pos.x - HALF_W, pos.y, pos.z - HALF_W),
            Vec3::new(pos.x + HALF_W, pos.y + HEIGHT, pos.z + HALF_W),
        )
    }

    /// Intenta desplazar `amount` en `axis` (0=X,1=Y,2=Z); si la caja chocaría
    /// con un sólido, cancela el movimiento en ese eje y anula la velocidad
    /// (marcando `on_ground` si fue al bajar).
    fn move_axis(&mut self, grid: &VoxelGrid, axis: usize, amount: f32) {
        if amount == 0.0 {
            return;
        }
        let mut np = self.pos;
        np[axis] += amount;
        let (min, max) = self.aabb_at(np);
        if aabb_hits_solid(grid, min, max) {
            if axis == 1 && amount < 0.0 {
                self.on_ground = true;
            }
            self.vel[axis] = 0.0;
        } else {
            self.pos = np;
        }
    }
}

/// `true` si la caja `[min, max]` solapa algún voxel sólido del grid. Recorre
/// las celdas enteras que toca la caja (cada voxel `(i,j,k)` ocupa
/// `[i,i+1)³`).
fn aabb_hits_solid(grid: &VoxelGrid, min: Vec3, max: Vec3) -> bool {
    let x0 = min.x.floor() as i32;
    let y0 = min.y.floor() as i32;
    let z0 = min.z.floor() as i32;
    // `-EPS` para que tocar exacto el plano del voxel siguiente no lo cuente.
    let x1 = (max.x - EPS).floor() as i32;
    let y1 = (max.y - EPS).floor() as i32;
    let z1 = (max.z - EPS).floor() as i32;
    for z in z0..=z1 {
        for y in y0..=y1 {
            for x in x0..=x1 {
                if grid.is_solid(x, y, z) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Piso sólido en `y=0` (una capa) sobre un grid 8³.
    fn grid_con_piso() -> VoxelGrid {
        let mut g = VoxelGrid::new([8, 8, 8]);
        for z in 0..8 {
            for x in 0..8 {
                g.set(x, 0, z, [100, 100, 100]);
            }
        }
        g
    }

    #[test]
    fn cae_y_se_apoya_en_el_piso() {
        let g = grid_con_piso();
        // Arranca flotando bien arriba.
        let mut p = Player::new(Vec3::new(4.5, 5.0, 4.5));
        for _ in 0..240 {
            p.step(&g, Vec3::ZERO, false, 1.0 / 60.0);
        }
        // El piso ocupa [0,1); los pies deben quedar apoyados a y≈1.
        assert!(p.on_ground, "debería terminar en el suelo");
        assert!((p.pos.y - 1.0).abs() < 0.05, "pies en y≈1, dio {}", p.pos.y);
    }

    #[test]
    fn una_pared_frena_el_avance() {
        let mut g = grid_con_piso();
        // Muro en x=6, toda la altura.
        for y in 0..8 {
            for z in 0..8 {
                g.set(6, y, z, [80, 80, 80]);
            }
        }
        let mut p = Player::new(Vec3::new(4.5, 1.0, 4.5));
        // Camina hacia +X contra el muro un buen rato.
        for _ in 0..120 {
            p.step(&g, Vec3::X, false, 1.0 / 60.0);
        }
        // No puede atravesar: su borde derecho (pos.x + HALF_W) queda en x<6.
        assert!(p.pos.x + HALF_W <= 6.0 + EPS, "atravesó el muro: x={}", p.pos.x);
        assert!(p.pos.x > 4.5, "debería haber avanzado algo hasta el muro");
    }

    #[test]
    fn salta_solo_desde_el_suelo() {
        let g = grid_con_piso();
        let mut p = Player::new(Vec3::new(4.5, 1.0, 4.5));
        // Un paso para asentar on_ground.
        p.step(&g, Vec3::ZERO, false, 1.0 / 60.0);
        assert!(p.on_ground);
        // Salta: la velocidad vertical se vuelve positiva y despega.
        p.step(&g, Vec3::ZERO, true, 1.0 / 60.0);
        assert!(p.vel.y > 0.0, "el salto debe dar velocidad ascendente");
        assert!(p.pos.y > 1.0, "debe despegar del piso");
        // En el aire, un segundo intento de salto no reinyecta velocidad.
        let antes = p.vel.y;
        p.step(&g, Vec3::ZERO, true, 1.0 / 60.0);
        assert!(p.vel.y < antes, "no debe re-saltar en el aire");
    }
}
