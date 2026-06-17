//! `Actor` — un **muñeco de cajas articuladas** (humanoide voxel estilo
//! Minecraft/MagicaVoxel) para *actuar* en una escena filmada. Es el tercer
//! ingrediente de la rama de juego (tras [`Player`](crate::Player) y
//! [`raycast`](crate::raycast)): un personaje **posable y animable** —
//! cabeza/torso/2 brazos/2 piernas, cada miembro una caja que rota en su
//! articulación (cadera/hombro), con un ciclo de **caminata** procedural.
//!
//! No toca la GPU: produce una **malla** (`Vec<Vertex3d>` + índices) en espacio
//! local del cuerpo (pies en el origen, mirando a `+Z`), que la app sube a un
//! [`Renderer3d`](llimphi_3d::Renderer3d) por frame (`set_geometry`) y compone
//! con los voxels en [`Scene3d`](llimphi_3d::Scene3d) — así el actor se **ocluye
//! correctamente** con el terreno. Reusable por cualquier película/juego voxel.

use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{push_cube, Vertex3d};

/// Cadencia del ciclo de caminata (rad/seg de fase): a mayor, pasos más rápidos.
const CADENCE: f32 = 8.0;
/// Amplitud de balanceo de miembros al caminar (rad).
const SWING: f32 = 0.7;

/// Personaje articulado. `pos` es el **centro de los pies** en espacio de mundo
/// (las mismas coordenadas del terreno/grid); `facing` el rumbo (yaw, `0`=`+Z`).
/// Los colores son por zona (piel/remera/pantalón) para leer las partes.
#[derive(Debug, Clone, Copy)]
pub struct Actor {
    /// Centro de los pies, en mundo.
    pub pos: Vec3,
    /// Rumbo (yaw, radianes; `0` mira a `+Z`).
    pub facing: f32,
    /// Fase del ciclo de caminata (acumulada por [`advance`](Self::advance)).
    pub phase: f32,
    /// Color de la piel (cabeza).
    pub skin: [f32; 3],
    /// Color de la remera (torso + brazos).
    pub shirt: [f32; 3],
    /// Color del pantalón (piernas).
    pub pants: [f32; 3],
}

impl Actor {
    /// Actor parado en `pos` (centro de pies, mundo) mirando a `facing`, con una
    /// paleta por defecto (piel clara, remera teal, pantalón azul).
    pub fn new(pos: Vec3, facing: f32) -> Self {
        Self {
            pos,
            facing,
            phase: 0.0,
            skin: [0.86, 0.68, 0.54],
            shirt: [0.20, 0.62, 0.55],
            pants: [0.18, 0.22, 0.34],
        }
    }

    /// Tinta el actor (piel/remera/pantalón) — encadenable tras [`new`](Self::new).
    pub fn with_colors(mut self, skin: [f32; 3], shirt: [f32; 3], pants: [f32; 3]) -> Self {
        self.skin = skin;
        self.shirt = shirt;
        self.pants = pants;
        self
    }

    /// Avanza la animación `dt` segundos. Si `moving`, hace girar la fase del
    /// ciclo de caminata (balancea miembros); quieto, deja la fase como está
    /// (postura actual congelada). El movimiento de `pos`/`facing` lo maneja el
    /// llamador (la dirección/guion).
    pub fn advance(&mut self, dt: f32, moving: bool) {
        if moving {
            self.phase += dt * CADENCE;
        }
    }

    /// Orienta al actor para mirar hacia `target` (sólo el plano horizontal).
    pub fn face_towards(&mut self, target: Vec3) {
        let d = target - self.pos;
        if d.x.abs() + d.z.abs() > 1e-4 {
            self.facing = d.x.atan2(d.z); // yaw=0 → +Z, consistente con forward_h
        }
    }

    /// Matriz de ubicación en mundo: traslada a `pos` y rota por `facing`. La
    /// malla de [`mesh`](Self::mesh) está en espacio local; este es el `model`
    /// del [`Renderer3d`](llimphi_3d::Renderer3d).
    pub fn model(&self) -> Mat4 {
        Mat4::from_translation(self.pos) * Mat4::from_rotation_y(self.facing)
    }

    /// Construye la **malla del cuerpo** en espacio local (pies en el origen,
    /// mirando a `+Z`) para la pose actual (fase de caminata). 6 cajas:
    /// cabeza, torso, 2 brazos y 2 piernas; brazos y piernas se balancean en
    /// oposición (el patrón de un paso). Subir con `Renderer3d::set_geometry` y
    /// ubicar con [`model`](Self::model).
    pub fn mesh(&self) -> (Vec<Vertex3d>, Vec<u16>) {
        let mut v = Vec::with_capacity(8 * 6);
        let mut i = Vec::with_capacity(36 * 6);

        let s = self.phase.sin() * SWING;

        // Torso (de y=0.8 a 1.4) y cabeza (encima). Sin articular.
        box_at(&mut v, &mut i, Vec3::new(0.0, 1.10, 0.0), Vec3::new(0.55, 0.60, 0.30), self.shirt);
        box_at(&mut v, &mut i, Vec3::new(0.0, 1.62, 0.0), Vec3::new(0.42, 0.40, 0.42), self.skin);

        // Piernas: cajas que cuelgan de la cadera (y=0.8) y rotan en X. Una
        // adelanta mientras la otra atrasa (`s` y `-s`).
        limb(&mut v, &mut i, Vec3::new(0.14, 0.80, 0.0), 0.80, Vec3::new(0.22, 0.80, 0.22), s, self.pants);
        limb(&mut v, &mut i, Vec3::new(-0.14, 0.80, 0.0), 0.80, Vec3::new(0.22, 0.80, 0.22), -s, self.pants);

        // Brazos: cuelgan del hombro (y=1.40), balanceo opuesto a la pierna del
        // mismo lado (brazo izq con pierna der → naturalidad del andar).
        limb(&mut v, &mut i, Vec3::new(0.36, 1.40, 0.0), 0.60, Vec3::new(0.18, 0.60, 0.18), -s, self.shirt);
        limb(&mut v, &mut i, Vec3::new(-0.36, 1.40, 0.0), 0.60, Vec3::new(0.18, 0.60, 0.18), s, self.shirt);

        (v, i)
    }
}

/// Apila una caja **estática** centrada en `center` con tamaño total `size`.
fn box_at(v: &mut Vec<Vertex3d>, i: &mut Vec<u16>, center: Vec3, size: Vec3, color: [f32; 3]) {
    let m = Mat4::from_translation(center) * Mat4::from_scale(size);
    push_cube(v, i, m, color);
}

/// Apila un **miembro articulado**: una caja de tamaño `size` y largo `len` que
/// cuelga del pivote `joint` (su extremo superior) y rota `angle` rad en torno
/// al eje X (balanceo adelante/atrás). El centro de la caja queda a `len/2` por
/// debajo del pivote antes de rotar.
fn limb(
    v: &mut Vec<Vertex3d>,
    i: &mut Vec<u16>,
    joint: Vec3,
    len: f32,
    size: Vec3,
    angle: f32,
    color: [f32; 3],
) {
    let m = Mat4::from_translation(joint)
        * Mat4::from_rotation_x(angle)
        * Mat4::from_translation(Vec3::new(0.0, -len / 2.0, 0.0))
        * Mat4::from_scale(size);
    push_cube(v, i, m, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malla_tiene_seis_cajas() {
        let a = Actor::new(Vec3::ZERO, 0.0);
        let (v, idx) = a.mesh();
        assert_eq!(v.len(), 8 * 6, "6 cajas × 8 vértices");
        assert_eq!(idx.len(), 36 * 6, "6 cajas × 36 índices");
    }

    #[test]
    fn caminar_balancea_las_piernas() {
        // A fase π/2 el seno es máximo → las piernas separan al máximo.
        let mut a = Actor::new(Vec3::ZERO, 0.0);
        a.advance(std::f32::consts::FRAC_PI_2 / CADENCE, true);
        let z: Vec<f32> = a.mesh().0.iter().map(|v| v.pos[2]).collect();
        let span = z.iter().cloned().fold(f32::MIN, f32::max)
            - z.iter().cloned().fold(f32::MAX, f32::min);
        assert!(span > 0.5, "al caminar los miembros deben adelantar/atrasar: span={span}");
    }

    #[test]
    fn quieto_no_avanza_la_fase() {
        let mut a = Actor::new(Vec3::ZERO, 0.0);
        a.advance(1.0, false);
        assert_eq!(a.phase, 0.0);
    }

    #[test]
    fn face_towards_mira_a_mas_z() {
        let mut a = Actor::new(Vec3::ZERO, 0.0);
        a.face_towards(Vec3::new(0.0, 0.0, 5.0));
        assert!(a.facing.abs() < 1e-4, "mirar a +Z → yaw≈0");
    }
}
