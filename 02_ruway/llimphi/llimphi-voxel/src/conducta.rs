//! `conducta` — **capa 3 del movimiento**: cómo un Ser *decide* moverse por el
//! mundo. Una [`Conducta`] son parámetros de *steering* (declarativos, como las Leyes
//! y los Andares); un [`Habitante`] es el agente que los ejecuta sobre la física de
//! cuerpo del [`Player`] (gravedad + colisión), combinando **deambular** (rumbo que
//! cambia con la inquietud), **gregarismo** (se acerca a su manada) y **miedo** (huye
//! de una amenaza). Determinista (LCG por agente; sin `rand`).
//!
//! Es CPU puro: el agente expone `pos`/`heading`/`fase`, y la app lo pinta con el
//! cuerpo del Ser (rig + andar) vía [`CharSpec::to_meta`](crate::CharSpec::to_meta).

use llimphi_3d::glam::Vec3;
use llimphi_3d::VoxelGrid;
use serde::{Deserialize, Serialize};

use crate::{forward_h, Player};

const TAU: f32 = std::f32::consts::TAU;
/// Radio (voxels) en el que un habitante "ve" a su manada y a la amenaza.
const RADIO_VISION: f32 = 12.0;

/// **Conducta** de un Ser: parámetros de locomoción, todos `[0,1]` salvo la
/// velocidad. Declarativa y editable; la ejecuta un [`Habitante`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Conducta {
    /// Velocidad de marcha (voxels/seg).
    pub velocidad: f32,
    /// **Inquietud** `[0,1]`: cuán seguido y fuerte cambia de rumbo al deambular.
    pub inquietud: f32,
    /// Probabilidad `[0,1]` de pegar un salto al caminar.
    pub salto: f32,
    /// **Gregarismo** `[0,1]`: cuánto se acerca al centro de su manada.
    pub gregario: f32,
    /// **Miedo** `[0,1]`: cuánto huye de una amenaza (jugador/depredador).
    pub miedo: f32,
}

impl Default for Conducta {
    fn default() -> Self {
        Self { velocidad: 3.0, inquietud: 0.4, salto: 0.08, gregario: 0.35, miedo: 0.6 }
    }
}

impl Conducta {
    /// Parámetros editables: `(nombre, valor, min, max)` — para armar sliders.
    pub fn params(&self) -> [(&'static str, f32, f32, f32); 5] {
        [
            ("velocidad", self.velocidad, 0.0, 6.0),
            ("inquietud", self.inquietud, 0.0, 1.0),
            ("salto", self.salto, 0.0, 1.0),
            ("gregario", self.gregario, 0.0, 1.0),
            ("miedo", self.miedo, 0.0, 1.0),
        ]
    }
    /// Fija el parámetro `i` (en el orden de [`params`](Self::params)).
    pub fn set(&mut self, i: usize, v: f32) {
        match i {
            0 => self.velocidad = v.clamp(0.0, 6.0),
            1 => self.inquietud = v.clamp(0.0, 1.0),
            2 => self.salto = v.clamp(0.0, 1.0),
            3 => self.gregario = v.clamp(0.0, 1.0),
            4 => self.miedo = v.clamp(0.0, 1.0),
            _ => {}
        }
    }
}

/// Un **habitante**: un agente que ejecuta una [`Conducta`] sobre la física del
/// [`Player`]. La app lo pinta con el cuerpo del Ser que representa.
#[derive(Debug, Clone)]
pub struct Habitante {
    body: Player,
    conducta: Conducta,
    /// Rumbo actual (yaw, rad; `0` mira a `+Z`).
    heading: f32,
    /// Fase de animación (avanza al caminar) para el andar del cuerpo.
    pub fase: f32,
    /// Cuenta atrás hasta el próximo cambio de rumbo (deambular).
    timer: f32,
    /// Pide saltar el próximo paso.
    jump: bool,
    /// LCG (azar determinista por habitante).
    rng: u32,
}

impl Habitante {
    /// Habitante posado sobre la columna `(x, z)` del grid, con su conducta y semilla.
    pub fn spawn(grid: &VoxelGrid, x: u32, z: u32, conducta: Conducta, seed: u32) -> Self {
        let mut body = Player::spawn_on(grid, x, z);
        body.speed = conducta.velocidad;
        let mut h = Self {
            body,
            conducta,
            heading: 0.0,
            fase: 0.0,
            timer: 0.0,
            jump: false,
            rng: seed | 1,
        };
        h.heading = h.next() * TAU;
        h
    }

    /// Posición (espacio de grilla, igual que el terreno).
    pub fn pos(&self) -> Vec3 {
        self.body.pos
    }
    /// Rumbo actual (para orientar el cuerpo).
    pub fn heading(&self) -> f32 {
        self.heading
    }

    /// Actualiza la conducta en caliente (para reflejar ediciones sin re-spawnear).
    pub fn set_conducta(&mut self, c: Conducta) {
        self.conducta = c;
    }

    /// Avanza `dt`: combina deambular + gregarismo (hacia el centroide de `vecinos`
    /// cercanos) + miedo (lejos de `amenaza`), y mueve el cuerpo. `vecinos` son las
    /// posiciones de los demás habitantes (la propia se ignora por distancia ~0).
    pub fn step(&mut self, grid: &VoxelGrid, vecinos: &[Vec3], amenaza: Option<Vec3>, dt: f32) {
        let c = self.conducta;

        // Deambular: cada tanto, empujar el rumbo (más inquietud = más seguido/fuerte).
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.heading += (self.next() - 0.5) * (0.5 + 2.0 * c.inquietud);
            self.timer = (1.4 - c.inquietud).max(0.25) + self.next() * 1.5;
            self.jump = self.next() < c.salto;
        }

        // Dirección deseada: rumbo de deambular + cohesión + huida.
        let pos = self.body.pos;
        let mut desired = forward_h(self.heading);

        if c.gregario > 0.0 && !vecinos.is_empty() {
            let mut centro = Vec3::ZERO;
            let mut n = 0.0;
            for &v in vecinos {
                let d = (v - pos).length();
                if d > 0.3 && d < RADIO_VISION {
                    centro += v;
                    n += 1.0;
                }
            }
            if n > 0.0 {
                let hacia = horiz_norm((centro / n) - pos);
                desired += hacia * (c.gregario * 1.5);
            }
        }

        if let Some(a) = amenaza {
            let d = (pos - a).length();
            if d > 0.01 && d < RADIO_VISION {
                // Más cerca = más fuerte la huida.
                let cerca = 1.0 - d / RADIO_VISION;
                desired += horiz_norm(pos - a) * (c.miedo * 2.5 * cerca);
            }
        }

        // Girar el rumbo hacia la dirección deseada (suave, con tope por paso).
        let desired = horiz_norm(desired);
        if desired.length_squared() > 1e-4 {
            let objetivo = desired.x.atan2(desired.z);
            let giro_max = (4.0 + 6.0 * c.inquietud) * dt;
            self.heading = acercar_angulo(self.heading, objetivo, giro_max);
        }

        // Mover el cuerpo; rebotar si choca (no quedar empujando una pared).
        self.body.speed = c.velocidad;
        let before = self.body.pos;
        let jump = self.jump && self.body.on_ground;
        self.body.step(grid, forward_h(self.heading), jump, dt);
        self.jump = false;

        let movido = (self.body.pos - before).length();
        if self.body.on_ground && movido < 0.2 * c.velocidad * dt {
            self.heading += TAU * 0.5 + (self.next() - 0.5);
            self.timer = 0.5;
        }
        // La fase de animación avanza con el movimiento (para el andar de caminata).
        self.fase += dt * (3.0 + c.velocidad);
    }

    /// Próximo `f32` en `[0,1)` del LCG.
    fn next(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// Normaliza un vector en el plano horizontal (Y=0); cero si es ~nulo.
fn horiz_norm(v: Vec3) -> Vec3 {
    let h = Vec3::new(v.x, 0.0, v.z);
    let l = h.length();
    if l > 1e-5 {
        h / l
    } else {
        Vec3::ZERO
    }
}

/// Acerca `a` hacia `b` (ángulos, rad) a lo sumo `max` por paso, por el lado corto.
fn acercar_angulo(a: f32, b: f32, max: f32) -> f32 {
    let mut d = (b - a) % TAU;
    if d > std::f32::consts::PI {
        d -= TAU;
    }
    if d < -std::f32::consts::PI {
        d += TAU;
    }
    a + d.clamp(-max, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arena 48×48 con piso y **paredes** perimetrales (corral): los habitantes
    /// rebotan y no se caen por el borde.
    fn grid_con_piso() -> VoxelGrid {
        let n = 48u32;
        let mut g = VoxelGrid::new([n, 8, n]);
        for z in 0..n {
            for x in 0..n {
                g.set(x, 0, z, [90, 140, 80]);
                if x == 0 || x == n - 1 || z == 0 || z == n - 1 {
                    for y in 1..6 {
                        g.set(x, y, z, [120, 120, 120]);
                    }
                }
            }
        }
        g
    }

    #[test]
    fn deambula_y_no_atraviesa_el_piso() {
        let g = grid_con_piso();
        let mut h = Habitante::spawn(&g, 24, 24, Conducta::default(), 7);
        let start = h.pos();
        for _ in 0..600 {
            h.step(&g, &[], None, 1.0 / 60.0);
            assert!(h.pos().y >= 1.0 - 0.05, "se hundió: y={}", h.pos().y);
        }
        assert!((h.pos() - start).length() > 1.0, "no deambuló");
    }

    #[test]
    fn gregario_los_junta_mas_que_indiferentes() {
        let g = grid_con_piso();
        // Dos pares lejos entre sí; uno gregario, otro no. Tras un rato, los gregarios
        // quedan más cerca entre sí que los indiferentes.
        let separacion = |gregario: f32, seed: u32| {
            let mut c = Conducta::default();
            c.gregario = gregario;
            c.inquietud = 0.2;
            let mut a = Habitante::spawn(&g, 18, 24, c, seed);
            let mut b = Habitante::spawn(&g, 30, 24, c, seed + 100);
            for _ in 0..900 {
                let pa = a.pos();
                let pb = b.pos();
                a.step(&g, &[pb], None, 1.0 / 60.0);
                b.step(&g, &[pa], None, 1.0 / 60.0);
            }
            (a.pos() - b.pos()).length()
        };
        let juntos = separacion(1.0, 1);
        let sueltos = separacion(0.0, 1);
        assert!(juntos < sueltos, "gregarios más cerca ({juntos:.1}) que sueltos ({sueltos:.1})");
    }

    #[test]
    fn miedo_aleja_de_la_amenaza() {
        let g = grid_con_piso();
        let amenaza = Vec3::new(24.5, 1.0, 24.5);
        let mut c = Conducta::default();
        c.miedo = 1.0;
        c.inquietud = 0.1;
        let mut h = Habitante::spawn(&g, 22, 24, c, 5); // arranca cerca de la amenaza
        let d0 = (h.pos() - amenaza).length();
        for _ in 0..600 {
            h.step(&g, &[], Some(amenaza), 1.0 / 60.0);
        }
        let d1 = (h.pos() - amenaza).length();
        assert!(d1 > d0 + 2.0, "huyó de la amenaza: {d0:.1} → {d1:.1}");
    }
}
