//! `CameraTrack` — interpolación de cámara por **keyframes** en el tiempo, el
//! ingrediente "cine" del motor: en vez de una `Camera3d` fija o atada a input,
//! una secuencia de poses `(t, eye, target, fov)` que se interpolan suave para
//! producir un **movimiento de cámara guionado** (travelling, grúa, dolly,
//! corte). Determinista por construcción → ideal para *filmar* frame a frame.
//!
//! Es genérico del motor 3D (no sabe de voxels ni de juegos): cualquier app que
//! quiera una cámara animada lo usa. La *dirección* de actores/eventos vive en
//! la capa de contenido (la app), no acá.

use glam::Vec3;

use crate::camera::Camera3d;

/// Una pose de cámara anclada a un instante `t` (segundos). Entre keys
/// consecutivas, [`CameraTrack::sample`] interpola `eye`/`target`/`fovy_rad`.
#[derive(Debug, Clone, Copy)]
pub struct CamKey {
    /// Instante de la pose, en segundos desde el inicio.
    pub t: f32,
    /// Posición del ojo.
    pub eye: Vec3,
    /// Punto al que mira.
    pub target: Vec3,
    /// Campo de visión vertical (radianes) en esta pose.
    pub fovy_rad: f32,
}

impl CamKey {
    /// Atajo: una pose mirando de `eye` a `target` con FOV en **grados**.
    pub fn look(t: f32, eye: Vec3, target: Vec3, fov_deg: f32) -> Self {
        Self { t, eye, target, fovy_rad: fov_deg.to_radians() }
    }
}

/// Secuencia de [`CamKey`] ordenada en el tiempo. `sample(t)` devuelve la
/// `Camera3d` interpolada; fuera de rango hace *clamp* a la primera/última pose.
#[derive(Debug, Clone, Default)]
pub struct CameraTrack {
    keys: Vec<CamKey>,
}

impl CameraTrack {
    /// Crea el track a partir de las keys (se ordenan por `t`). Un track vacío
    /// o de una sola key es válido (devuelve siempre esa pose).
    pub fn new(mut keys: Vec<CamKey>) -> Self {
        keys.sort_by(|a, b| a.t.total_cmp(&b.t));
        Self { keys }
    }

    /// Duración total (el `t` de la última key), o `0.0` si está vacío.
    pub fn duration(&self) -> f32 {
        self.keys.last().map(|k| k.t).unwrap_or(0.0)
    }

    /// La cámara interpolada en el instante `t` (segundos). Entre dos keys usa
    /// **smoothstep** (acelera/desacelera suave, sin tirones) sobre la fracción
    /// del segmento; antes de la primera / después de la última, clampa.
    pub fn sample(&self, t: f32) -> Camera3d {
        match self.keys.as_slice() {
            [] => Camera3d::default(),
            [only] => cam_of(only),
            keys => {
                // Clamp a los extremos.
                if t <= keys[0].t {
                    return cam_of(&keys[0]);
                }
                if t >= keys[keys.len() - 1].t {
                    return cam_of(&keys[keys.len() - 1]);
                }
                // Segmento que contiene a `t`: última key con `t_key <= t`
                // (existe y no es la última, por el clamp de arriba).
                let i = keys.iter().rposition(|k| k.t <= t).unwrap_or(0).min(keys.len() - 2);
                let (a, b) = (&keys[i], &keys[i + 1]);
                let span = (b.t - a.t).max(1e-6);
                let f = smoothstep((t - a.t) / span);
                Camera3d {
                    eye: a.eye.lerp(b.eye, f),
                    target: a.target.lerp(b.target, f),
                    fovy_rad: a.fovy_rad + (b.fovy_rad - a.fovy_rad) * f,
                    ..Camera3d::default()
                }
            }
        }
    }
}

/// Construye una `Camera3d` (con `up`/planos por defecto) desde una key.
fn cam_of(k: &CamKey) -> Camera3d {
    Camera3d {
        eye: k.eye,
        target: k.target,
        fovy_rad: k.fovy_rad,
        ..Camera3d::default()
    }
}

/// Suavizado Hermite clásico `3t²−2t³` en `[0,1]` (deriva nula en los extremos).
fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> CameraTrack {
        CameraTrack::new(vec![
            CamKey::look(0.0, Vec3::new(0.0, 0.0, 0.0), Vec3::Z, 60.0),
            CamKey::look(2.0, Vec3::new(10.0, 0.0, 0.0), Vec3::Z, 40.0),
        ])
    }

    #[test]
    fn clamp_en_los_extremos() {
        let tr = track();
        assert_eq!(tr.sample(-1.0).eye, Vec3::ZERO);
        assert_eq!(tr.sample(5.0).eye.x, 10.0);
        assert_eq!(tr.duration(), 2.0);
    }

    #[test]
    fn interpola_la_mitad_con_smoothstep() {
        let tr = track();
        // En la mitad temporal, smoothstep(0.5)=0.5 → punto medio exacto.
        let c = tr.sample(1.0);
        assert!((c.eye.x - 5.0).abs() < 1e-4, "x={}", c.eye.x);
        assert!((c.fovy_rad - 50_f32.to_radians()).abs() < 1e-4);
    }

    #[test]
    fn smoothstep_acelera_suave() {
        let tr = track();
        // A 1/4 del tiempo, smoothstep(0.25)=0.15625 < 0.25 (arranca lento).
        let c = tr.sample(0.5);
        assert!(c.eye.x < 2.5, "debería ir más lento al principio: x={}", c.eye.x);
    }
}
