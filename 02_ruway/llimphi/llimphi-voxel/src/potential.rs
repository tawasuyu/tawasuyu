//! **Objeto potencial**: una cosa colocada en el mundo en su forma *latente* que,
//! ante un disparador (acá: el aterrizaje/nacimiento), **eclosiona** en la entidad
//! que será. El primero es el [`Egg`] — un *huevo* que lleva adentro su potencial
//! ([`Hatchling`]: qué nace, con qué edad y colores) y al abrirse **da a luz** un
//! [`Actor`] recién nacido.
//!
//! Es la pieza del corto: la cámara cae, ve el huevo, y al tocar suelo el huevo se
//! abre y nace el niño. La forma generaliza (otros objetos potenciales podrían nacer
//! plantas, animales, estructuras); hoy el caso es el *huevito humano*.

use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{push_cube, Vertex3d};

use crate::actor::{Actor, Age};

/// `T(center) · S(size)` — caja centrada en `center` escalada a `size`.
fn trs(center: Vec3, size: Vec3) -> Mat4 {
    Mat4::from_translation(center) * Mat4::from_scale(size)
}

/// **Lo que un huevo va a ser**: el potencial latente. Hoy un personaje (edad +
/// colores); el motor lo materializa como [`Actor`] al nacer.
#[derive(Debug, Clone, Copy)]
pub struct Hatchling {
    /// Edad con la que nace (el corto: [`Age::Baby`]).
    pub age: Age,
    pub skin: [f32; 3],
    pub shirt: [f32; 3],
    pub pants: [f32; 3],
}

impl Hatchling {
    /// Un **huevito humano**: nace bebé, con una paleta cálida por defecto.
    pub fn human(age: Age) -> Self {
        Self {
            age,
            skin: [0.90, 0.74, 0.60],
            shirt: [0.86, 0.46, 0.44],
            pants: [0.22, 0.24, 0.32],
        }
    }
}

/// **Huevo** — un objeto potencial con forma de ovoide voxel. `hatch` va de `0`
/// (intacto) a `1` (abierto: la tapa superior se levanta y se inclina, dejando
/// salir al recién nacido). `pos` es la **base** del huevo en mundo.
#[derive(Debug, Clone, Copy)]
pub struct Egg {
    /// Base del huevo, en mundo (mismas coords que el terreno/actor).
    pub pos: Vec3,
    /// Rumbo (yaw) — orienta al recién nacido y la apertura.
    pub facing: f32,
    /// Altura del huevo.
    pub size: f32,
    /// Color de la cáscara.
    pub shell: [f32; 3],
    /// Progreso de eclosión `[0,1]` (0 = intacto, 1 = abierto).
    pub hatch: f32,
    /// Qué nace de él.
    pub becomes: Hatchling,
}

/// Fracción de la altura donde el huevo se "raja" (la tapa de arriba es la que se
/// abre).
const CRACK: f32 = 0.55;
/// Slabs verticales con los que se aproxima el ovoide.
const SLABS: usize = 9;

impl Egg {
    /// Huevo intacto en `pos` (base) de altura `size`, con el potencial `becomes`.
    pub fn new(pos: Vec3, size: f32, becomes: Hatchling) -> Self {
        Self { pos, facing: 0.0, size, shell: [0.94, 0.92, 0.86], hatch: 0.0, becomes }
    }

    /// Avanza la eclosión `dt` segundos a `rate` (`1/seg` ⇒ se abre en ~1 s).
    pub fn advance(&mut self, dt: f32, rate: f32) {
        self.hatch = (self.hatch + dt * rate).clamp(0.0, 1.0);
    }

    /// `true` cuando el huevo terminó de abrirse.
    pub fn is_open(&self) -> bool {
        self.hatch >= 1.0
    }

    /// Matriz de ubicación en mundo (igual rol que [`Actor::model`]).
    pub fn model(&self) -> Mat4 {
        Mat4::from_translation(self.pos) * Mat4::from_rotation_y(self.facing)
    }

    /// **Da a luz**: materializa el potencial como un [`Actor`] recién nacido, en la
    /// posición y rumbo del huevo, con la edad/colores de [`Hatchling`].
    pub fn newborn(&self) -> Actor {
        Actor::new(self.pos, self.facing)
            .with_age(self.becomes.age)
            .with_colors(self.becomes.skin, self.becomes.shirt, self.becomes.pants)
    }

    /// Malla del cascarón en espacio local (base en el origen, ubicar con
    /// [`model`](Self::model)). Ovoide por slabs horizontales; la tapa por encima de
    /// [`CRACK`] **se separa** (se levanta y se inclina) según `hatch`, bisagra en la
    /// línea de la rajadura.
    pub fn mesh(&self) -> (Vec<Vertex3d>, Vec<u16>) {
        let mut v = Vec::with_capacity(8 * SLABS);
        let mut i = Vec::with_capacity(36 * SLABS);
        let half_w = self.size * 0.42; // medio-ancho máximo (en la panza)
        let thick = self.size / SLABS as f32;
        let pivot = Vec3::new(0.0, CRACK * self.size, 0.0);

        // Transform de la tapa: bisagra en la rajadura, se levanta y se inclina.
        let lift = self.hatch * self.size * 0.75;
        let tilt = self.hatch * 0.9;
        let cap = Mat4::from_translation(pivot + Vec3::new(0.0, lift, 0.0))
            * Mat4::from_rotation_z(tilt)
            * Mat4::from_translation(-pivot);

        for k in 0..SLABS {
            let tc = (k as f32 + 0.5) / SLABS as f32; // centro del slab en [0,1]
            // Medio-ancho del ovoide (elipse): 0 en los polos, máx en la panza.
            let r = (half_w * (4.0 * tc * (1.0 - tc)).sqrt()).max(self.size * 0.04);
            let y = tc * self.size;
            let base = trs(Vec3::new(0.0, y, 0.0), Vec3::new(r * 2.0, thick * 1.04, r * 2.0));
            let m = if tc > CRACK { cap * base } else { base };
            push_cube(&mut v, &mut i, m, self.shell);
        }
        (v, i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_recien_nacido_hereda_edad_y_lugar() {
        let pos = Vec3::new(5.0, -2.0, 3.0);
        let egg = Egg::new(pos, 1.0, Hatchling::human(Age::Baby));
        let baby = egg.newborn();
        assert_eq!(baby.age, Age::Baby);
        assert_eq!(baby.pos, pos);
    }

    #[test]
    fn la_eclosion_avanza_y_se_satura() {
        let mut egg = Egg::new(Vec3::ZERO, 1.0, Hatchling::human(Age::Baby));
        assert!(!egg.is_open());
        egg.advance(0.5, 1.0);
        assert!((egg.hatch - 0.5).abs() < 1e-5);
        egg.advance(10.0, 1.0); // se pasa → se clampa
        assert!(egg.is_open() && egg.hatch == 1.0);
    }

    #[test]
    fn al_abrirse_la_tapa_se_levanta() {
        let intact = Egg::new(Vec3::ZERO, 2.0, Hatchling::human(Age::Baby));
        let mut open = intact;
        open.hatch = 1.0;
        let top = |e: &Egg| e.mesh().0.iter().map(|v| v.pos[1]).fold(f32::MIN, f32::max);
        assert!(top(&open) > top(&intact) + 0.5, "la tapa abierta queda más alta: {} vs {}", top(&open), top(&intact));
    }
}
