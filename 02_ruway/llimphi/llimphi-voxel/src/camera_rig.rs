//! **Modos de cámara** atados a un sujeto + la **secuencia de nacimiento** del
//! corto. Dos modos generales, reusables por cualquier juego/escena:
//! - [`CamMode::Subject`] — *primera persona*: la cámara **es** el sujeto (ojo en su
//!   cabeza, mira hacia su rumbo).
//! - [`CamMode::Follow`] — *tercera persona*: detrás y arriba del sujeto, mirándolo.
//!
//! Y [`BirthSequence`], que guiona la apertura: la cámara **cae del cielo mirando
//! abajo**, ve el huevo, y al aterrizar (nacer) **sale del sujeto** y se planta
//! detrás — una transición suave `Subject → Follow`.

use llimphi_3d::glam::Vec3;
use llimphi_3d::Camera3d;

use crate::actor::Actor;
use crate::player::{forward_h, look_dir};
use crate::potential::Egg;

/// Suavizado Hermite `3t²−2t³` (deriva nula en los extremos).
fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// **Modo de cámara** relativo a un sujeto (su `pos` = centro de pies, y su
/// `facing` = rumbo). [`camera`](Self::camera) produce la [`Camera3d`] del frame.
#[derive(Debug, Clone, Copy)]
pub enum CamMode {
    /// Primera persona: el ojo está a `eye_height` sobre los pies del sujeto y mira
    /// hacia su rumbo (con `pitch` de cabeceo). La cámara **es** el sujeto.
    Subject { eye_height: f32, pitch: f32 },
    /// Tercera persona: el ojo está `distance` detrás del sujeto y `height` arriba,
    /// mirándolo a la altura del pecho.
    Follow { distance: f32, height: f32 },
}

impl CamMode {
    /// La cámara de este modo para un sujeto en `pos` (pies) mirando a `facing`.
    pub fn camera(&self, pos: Vec3, facing: f32) -> Camera3d {
        match *self {
            CamMode::Subject { eye_height, pitch } => {
                let eye = pos + Vec3::new(0.0, eye_height, 0.0);
                let dir = look_dir(facing, pitch);
                Camera3d { eye, target: eye + dir, ..Camera3d::default() }
            }
            CamMode::Follow { distance, height } => {
                let look = pos + Vec3::new(0.0, height, 0.0);
                let behind = forward_h(facing) * distance;
                let eye = pos - behind + Vec3::new(0.0, height + 0.6, 0.0);
                Camera3d { eye, target: look, ..Camera3d::default() }
            }
        }
    }
}

/// Interpola dos cámaras (ojo, objetivo y FOV) con suavizado — la base de las
/// **transiciones** entre modos (p.ej. salir del sujeto hacia atrás).
pub fn cam_lerp(a: &Camera3d, b: &Camera3d, t: f32) -> Camera3d {
    let s = smoothstep(t);
    Camera3d {
        eye: a.eye.lerp(b.eye, s),
        target: a.target.lerp(b.target, s),
        up: Vec3::Y,
        fovy_rad: a.fovy_rad + (b.fovy_rad - a.fovy_rad) * s,
        znear: a.znear,
        zfar: a.zfar,
    }
}

/// **Secuencia de nacimiento**: la cámara cae del cielo sobre el huevo mirando
/// abajo; al aterrizar el huevo eclosiona y la cámara —que llegó *metida en el
/// sujeto*— sale hacia atrás a tercera persona. Determinista por tiempo
/// (reproducible cuadro a cuadro).
#[derive(Debug, Clone, Copy)]
pub struct BirthSequence {
    /// El huevo (lleva pos, rumbo y el potencial que nace).
    pub egg: Egg,
    /// Altura desde la que cae la cámara.
    pub sky_height: f32,
    /// Instante del aterrizaje/nacimiento (seg).
    pub t_land: f32,
    /// Duración de la salida del sujeto hacia atrás (seg).
    pub t_pull: f32,
    /// Distancia/altura del plano de seguimiento final.
    pub follow_distance: f32,
    pub follow_height: f32,
}

impl BirthSequence {
    /// Secuencia con tiempos por defecto (caída ~2.4 s, salida ~1.2 s).
    pub fn new(egg: Egg) -> Self {
        Self {
            egg,
            sky_height: 60.0,
            t_land: 2.4,
            t_pull: 1.2,
            follow_distance: 3.5,
            follow_height: 1.0,
        }
    }

    /// Duración total de la secuencia (hasta un pequeño respiro tras la salida).
    pub fn duration(&self) -> f32 {
        self.t_land + self.t_pull + 1.0
    }

    /// El recién nacido (materializa el potencial del huevo).
    pub fn newborn(&self) -> Actor {
        self.egg.newborn()
    }

    /// Progreso de eclosión del huevo en `t`: arranca poco antes del aterrizaje y
    /// completa poco después (el huevo se abre **mientras** la cámara aterriza).
    pub fn hatch(&self, t: f32) -> f32 {
        let start = self.t_land - 0.4;
        let end = self.t_land + 0.3;
        ((t - start) / (end - start)).clamp(0.0, 1.0)
    }

    /// Altura del ojo en primera persona = la cabeza del recién nacido.
    fn eye_height(&self) -> f32 {
        self.newborn().build.head_y
    }

    /// Cámara en primera persona parada en el sujeto (recién nacido).
    fn subject_cam(&self) -> Camera3d {
        CamMode::Subject { eye_height: self.eye_height(), pitch: 0.0 }
            .camera(self.egg.pos, self.egg.facing)
    }

    /// Cámara de seguimiento detrás del sujeto.
    fn follow_cam(&self) -> Camera3d {
        CamMode::Follow { distance: self.follow_distance, height: self.follow_height }
            .camera(self.egg.pos, self.egg.facing)
    }

    /// Cámara cayendo del cielo, mirando hacia abajo al huevo.
    fn sky_cam(&self) -> Camera3d {
        let eye = self.egg.pos + Vec3::new(0.0, self.sky_height, 0.01);
        Camera3d { eye, target: self.egg.pos, ..Camera3d::default() }
    }

    /// La cámara en el instante `t`: **caída** (cielo→sujeto, mirando abajo→al
    /// frente) hasta `t_land`; **salida** (sujeto→seguimiento) durante `t_pull`;
    /// luego, seguimiento fijo.
    pub fn camera(&self, t: f32) -> Camera3d {
        let subject = self.subject_cam();
        if t < self.t_land {
            // Cae del cielo y, al final, queda calzada en el sujeto.
            cam_lerp(&self.sky_cam(), &subject, (t / self.t_land.max(1e-3)).clamp(0.0, 1.0))
        } else if t < self.t_land + self.t_pull {
            // Sale del sujeto hacia atrás (3ª persona).
            let u = (t - self.t_land) / self.t_pull.max(1e-3);
            cam_lerp(&subject, &self.follow_cam(), u)
        } else {
            self.follow_cam()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::Age;
    use crate::potential::Hatchling;

    fn egg() -> Egg {
        let mut e = Egg::new(Vec3::new(0.0, 0.0, 0.0), 1.2, Hatchling::human(Age::Baby));
        e.facing = 0.0; // mira a +Z
        e
    }

    #[test]
    fn subject_mira_al_frente_desde_la_cabeza() {
        let cam = CamMode::Subject { eye_height: 1.6, pitch: 0.0 }.camera(Vec3::ZERO, 0.0);
        assert!((cam.eye.y - 1.6).abs() < 1e-5, "ojo en la cabeza");
        // Mira hacia +Z (rumbo 0).
        assert!((cam.target - cam.eye).z > 0.5, "mira al frente (+Z)");
    }

    #[test]
    fn follow_esta_detras_del_sujeto() {
        let cam = CamMode::Follow { distance: 4.0, height: 1.0 }.camera(Vec3::ZERO, 0.0);
        // Rumbo +Z → "detrás" es −Z; el ojo debe estar en z negativo.
        assert!(cam.eye.z < -1.0, "ojo detrás (−Z): {}", cam.eye.z);
        assert!(cam.eye.y > 1.0, "ojo por encima");
    }

    #[test]
    fn la_secuencia_cae_del_cielo_y_termina_atras() {
        let seq = BirthSequence::new(egg());
        let high = seq.camera(0.0);
        assert!(high.eye.y > 40.0, "arranca alto en el cielo: {}", high.eye.y);
        // Al final = plano de seguimiento (detrás del sujeto).
        let end = seq.camera(seq.duration());
        let follow = seq.camera(1e9);
        assert!((end.eye - follow.eye).length() < 1e-3, "termina en seguimiento");
        assert!(end.eye.z < 0.0, "el seguimiento final está detrás (−Z)");
        // La eclosión arranca cerrada y termina abierta.
        assert_eq!(seq.hatch(0.0), 0.0);
        assert_eq!(seq.hatch(seq.duration()), 1.0);
    }

    #[test]
    fn la_camara_es_continua_en_el_aterrizaje() {
        // Sin saltos bruscos al pasar de caída a salida (mismo punto en t_land).
        let seq = BirthSequence::new(egg());
        let before = seq.camera(seq.t_land - 0.001);
        let after = seq.camera(seq.t_land + 0.001);
        assert!((before.eye - after.eye).length() < 0.2, "ojo continuo en el aterrizaje");
    }
}
