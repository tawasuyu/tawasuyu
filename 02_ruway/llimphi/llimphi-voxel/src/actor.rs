//! `Actor` — un **muñeco de cajas articuladas** (humanoide voxel estilo
//! Minecraft/MagicaVoxel) para *actuar* en una escena filmada, con una pequeña
//! **librería de clips de animación** ([`Clip`]: quieto/caminar/correr/saludar/
//! señalar/festejar). Es el tercer ingrediente de la rama de juego (tras
//! [`Player`](crate::Player) y [`raycast`](crate::raycast)): un personaje
//! **posable y animable**.
//!
//! El cuerpo son 6 cajas (cabeza/torso/2 brazos/2 piernas); cada miembro rota en
//! su articulación (cadera/hombro). Un [`Clip`] es una función `fase → `[`Pose`]
//! (los ángulos de todas las articulaciones), así agregar una animación nueva es
//! escribir una pose, no tocar el render. No toca la GPU: produce una **malla**
//! (`Vec<Vertex3d>` + índices) en espacio local (pies en el origen, mirando a
//! `+Z`) que la app sube a un [`Renderer3d`](llimphi_3d::Renderer3d) por frame
//! (`set_geometry`) y compone con los voxels en [`Scene3d`](llimphi_3d::Scene3d).

use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{push_cube, Vertex3d};

/// Amplitud base de balanceo de miembros al caminar (rad).
const SWING: f32 = 0.7;
/// Duración del **cross-fade** al cambiar de clip (seg): el cuerpo mezcla la pose
/// saliente con la entrante en este lapso, en vez de saltar en seco.
const BLEND_DUR: f32 = 0.22;

/// Suavizado Hermite `3t²−2t³` (deriva nula en los extremos) para el cross-fade.
fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Ángulos de todas las articulaciones del muñeco en un instante. Una animación
/// ([`Clip`]) produce una `Pose`; [`Actor::mesh`] la hornea a cajas. Ángulos en
/// radianes; `0` = postura neutra (de pie, brazos colgando).
#[derive(Debug, Clone, Copy, Default)]
pub struct Pose {
    /// Balanceo de la pierna izquierda/derecha en la cadera (eje X, adelante+).
    pub leg_l: f32,
    pub leg_r: f32,
    /// Balanceo del brazo izquierdo/derecho en el hombro (eje X, adelante+).
    pub arm_l: f32,
    pub arm_r: f32,
    /// Apertura del brazo izquierdo/derecho hacia el costado/arriba (eje Z). El
    /// signo se espeja por lado dentro de [`Actor::mesh`]; positivo = levantar.
    pub arm_l_out: f32,
    pub arm_r_out: f32,
    /// Cabeceo de la cabeza (eje X).
    pub head_pitch: f32,
    /// Desplazamiento vertical del cuerpo (rebote/respiración), en unidades.
    pub bob: f32,
    /// Inclinación del torso hacia adelante (eje X, alrededor de los pies).
    pub lean: f32,
}

impl Pose {
    /// Interpola campo a campo entre dos poses (`t=0`→`a`, `t=1`→`b`). Lo usa el
    /// cross-fade entre clips.
    pub fn lerp(a: &Pose, b: &Pose, t: f32) -> Pose {
        let l = |x: f32, y: f32| x + (y - x) * t;
        Pose {
            leg_l: l(a.leg_l, b.leg_l),
            leg_r: l(a.leg_r, b.leg_r),
            arm_l: l(a.arm_l, b.arm_l),
            arm_r: l(a.arm_r, b.arm_r),
            arm_l_out: l(a.arm_l_out, b.arm_l_out),
            arm_r_out: l(a.arm_r_out, b.arm_r_out),
            head_pitch: l(a.head_pitch, b.head_pitch),
            bob: l(a.bob, b.bob),
            lean: l(a.lean, b.lean),
        }
    }
}

/// Animación: una función determinista `fase → `[`Pose`]. La fase la acumula
/// [`Actor::advance`] a la [`cadence`](Clip::cadence) del clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clip {
    /// De pie, respirando apenas.
    Idle,
    /// Caminata: piernas/brazos en oposición.
    Walk,
    /// Trote: balanceo amplio + inclinación hacia adelante.
    Run,
    /// Saludo: un brazo levantado al costado, oscilando.
    Wave,
    /// Señalar: un brazo extendido hacia adelante, firme.
    Point,
    /// Festejo: ambos brazos arriba, rebotando.
    Cheer,
}

impl Clip {
    /// Velocidad de avance de la fase (rad/seg): pasos más rápidos = más cadencia.
    pub fn cadence(self) -> f32 {
        match self {
            Clip::Idle => 2.0,
            Clip::Walk => 8.0,
            Clip::Run => 13.0,
            Clip::Wave => 9.0,
            Clip::Point => 3.0,
            Clip::Cheer => 7.0,
        }
    }

    /// La pose de este clip en la `fase` dada.
    pub fn pose(self, phase: f32) -> Pose {
        let s = phase.sin();
        match self {
            Clip::Idle => Pose {
                bob: 0.02 * s,
                head_pitch: 0.03 * s,
                arm_l_out: 0.07,
                arm_r_out: 0.07,
                ..Pose::default()
            },
            Clip::Walk => Pose {
                leg_l: s * SWING,
                leg_r: -s * SWING,
                arm_l: -s * SWING,
                arm_r: s * SWING,
                bob: (phase * 2.0).sin().abs() * 0.03,
                ..Pose::default()
            },
            Clip::Run => Pose {
                leg_l: s,
                leg_r: -s,
                arm_l: -s * 1.1,
                arm_r: s * 1.1,
                lean: 0.38,
                bob: (phase * 2.0).sin().abs() * 0.06,
                ..Pose::default()
            },
            Clip::Wave => Pose {
                arm_r_out: 2.35 + 0.18 * s, // levantado al costado, saludando
                arm_l_out: 0.08,
                head_pitch: -0.05,
                ..Pose::default()
            },
            Clip::Point => Pose {
                arm_r: -1.5, // extendido hacia adelante (+Z)
                arm_l_out: 0.07,
                head_pitch: 0.08,
                ..Pose::default()
            },
            Clip::Cheer => Pose {
                arm_l_out: 2.6,
                arm_r_out: 2.6,
                bob: (phase * 2.0).sin().abs() * 0.08,
                head_pitch: -0.1,
                ..Pose::default()
            },
        }
    }
}

/// Personaje articulado. `pos` es el **centro de los pies** en espacio de mundo
/// (las mismas coordenadas del terreno/grid); `facing` el rumbo (yaw, `0`=`+Z`).
/// `clip`/`phase` definen la animación actual. Colores por zona (piel/remera/
/// pantalón).
#[derive(Debug, Clone, Copy)]
pub struct Actor {
    /// Centro de los pies, en mundo.
    pub pos: Vec3,
    /// Rumbo (yaw, radianes; `0` mira a `+Z`).
    pub facing: f32,
    /// Animación actual.
    pub clip: Clip,
    /// Fase del clip (acumulada por [`advance`](Self::advance)).
    pub phase: f32,
    /// Clip saliente durante un cross-fade (`None` si no hay transición en curso).
    prev_clip: Option<Clip>,
    /// Fase del clip saliente (sigue avanzando durante la mezcla).
    prev_phase: f32,
    /// Progreso del cross-fade `0..1` (a `1` se descarta el clip saliente).
    blend: f32,
    /// Color de la piel (cabeza).
    pub skin: [f32; 3],
    /// Color de la remera (torso + brazos).
    pub shirt: [f32; 3],
    /// Color del pantalón (piernas).
    pub pants: [f32; 3],
}

impl Actor {
    /// Actor parado en `pos` (centro de pies, mundo) mirando a `facing`, en
    /// [`Clip::Idle`], con una paleta por defecto (piel clara, remera teal,
    /// pantalón azul).
    pub fn new(pos: Vec3, facing: f32) -> Self {
        Self {
            pos,
            facing,
            clip: Clip::Idle,
            phase: 0.0,
            prev_clip: None,
            prev_phase: 0.0,
            blend: 1.0,
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

    /// Cambia la animación. Si es un clip distinto, arranca un **cross-fade**: la
    /// pose saliente se mezcla con la nueva durante [`BLEND_DUR`] segundos (sin
    /// saltos). Repetir el mismo clip no corta nada.
    pub fn set_clip(&mut self, clip: Clip) {
        if self.clip != clip {
            self.prev_clip = Some(self.clip);
            self.prev_phase = self.phase;
            self.clip = clip;
            self.phase = 0.0;
            self.blend = 0.0;
        }
    }

    /// Avanza la animación `dt` segundos: la fase a la cadencia del clip, y —si hay
    /// transición— la fase saliente y el progreso del cross-fade. El movimiento de
    /// `pos`/`facing` lo maneja el llamador (la dirección).
    pub fn advance(&mut self, dt: f32) {
        self.phase += dt * self.clip.cadence();
        if let Some(pc) = self.prev_clip {
            self.prev_phase += dt * pc.cadence();
            self.blend += dt / BLEND_DUR;
            if self.blend >= 1.0 {
                self.prev_clip = None;
            }
        }
    }

    /// La pose actual del cuerpo: la del clip vigente, o —durante un cambio— la
    /// **mezcla** suave entre el clip saliente y el entrante.
    pub fn pose(&self) -> Pose {
        let target = self.clip.pose(self.phase);
        match self.prev_clip {
            Some(pc) => Pose::lerp(&pc.pose(self.prev_phase), &target, smoothstep(self.blend)),
            None => target,
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
    /// mirando a `+Z`) para la pose del clip/fase actuales. 6 cajas. El cuerpo
    /// superior (torso/cabeza/brazos) lleva el `bob`+`lean` de la pose; las
    /// piernas quedan plantadas (sólo su balanceo de cadera) para no levantar los
    /// pies del suelo. Subir con `Renderer3d::set_geometry` y ubicar con
    /// [`model`](Self::model).
    pub fn mesh(&self) -> (Vec<Vertex3d>, Vec<u16>) {
        let p = self.pose();
        let mut v = Vec::with_capacity(8 * 6);
        let mut i = Vec::with_capacity(36 * 6);

        // Transform del cuerpo superior: rebote vertical + inclinación adelante
        // (rotación en X alrededor de los pies/origen).
        let body = Mat4::from_translation(Vec3::new(0.0, p.bob, 0.0)) * Mat4::from_rotation_x(p.lean);

        // Torso y cabeza (la cabeza con su cabeceo, alrededor de su centro).
        push_cube(
            &mut v,
            &mut i,
            body * trs(Vec3::new(0.0, 1.10, 0.0), Mat4::IDENTITY, Vec3::new(0.55, 0.60, 0.30)),
            self.shirt,
        );
        push_cube(
            &mut v,
            &mut i,
            body * trs(Vec3::new(0.0, 1.62, 0.0), Mat4::from_rotation_x(p.head_pitch), Vec3::new(0.42, 0.40, 0.42)),
            self.skin,
        );

        // Piernas (sin `body`: pies plantados). Articulación en cadera y=0.8.
        limb(&mut v, &mut i, Mat4::IDENTITY, Vec3::new(0.14, 0.80, 0.0), 0.80, Vec3::new(0.22, 0.80, 0.22), Mat4::from_rotation_x(p.leg_r), self.pants);
        limb(&mut v, &mut i, Mat4::IDENTITY, Vec3::new(-0.14, 0.80, 0.0), 0.80, Vec3::new(0.22, 0.80, 0.22), Mat4::from_rotation_x(p.leg_l), self.pants);

        // Brazos (con `body`). Hombro y=1.40; rotación = apertura(Z)·balanceo(X).
        // La apertura se espeja por lado (positivo = levantar hacia su costado).
        let arm = Vec3::new(0.18, 0.60, 0.18);
        limb(&mut v, &mut i, body, Vec3::new(0.36, 1.40, 0.0), 0.60, arm, Mat4::from_rotation_z(p.arm_r_out) * Mat4::from_rotation_x(p.arm_r), self.shirt);
        limb(&mut v, &mut i, body, Vec3::new(-0.36, 1.40, 0.0), 0.60, arm, Mat4::from_rotation_z(-p.arm_l_out) * Mat4::from_rotation_x(p.arm_l), self.shirt);

        (v, i)
    }
}

/// `T(center) · R · S(size)` — caja centrada en `center`, rotada por `rot`,
/// escalada a `size` (un cubo unitario → su caja en el cuerpo).
fn trs(center: Vec3, rot: Mat4, size: Vec3) -> Mat4 {
    Mat4::from_translation(center) * rot * Mat4::from_scale(size)
}

/// Apila un **miembro articulado**: caja de tamaño `size` y largo `len` que
/// cuelga del pivote `joint` (su extremo superior) y rota por `rot` en torno a
/// ese pivote; todo prefijado por `pre` (el transform del cuerpo, o identidad
/// para las piernas). El centro de la caja queda a `len/2` por debajo del pivote
/// antes de rotar.
#[allow(clippy::too_many_arguments)]
fn limb(
    v: &mut Vec<Vertex3d>,
    i: &mut Vec<u16>,
    pre: Mat4,
    joint: Vec3,
    len: f32,
    size: Vec3,
    rot: Mat4,
    color: [f32; 3],
) {
    let m = pre
        * Mat4::from_translation(joint)
        * rot
        * Mat4::from_translation(Vec3::new(0.0, -len / 2.0, 0.0))
        * Mat4::from_scale(size);
    push_cube(v, i, m, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    /// Rango en Z de los vértices de la malla (cuánto adelantan/atrasan miembros).
    fn z_span(a: &Actor) -> f32 {
        let z: Vec<f32> = a.mesh().0.iter().map(|v| v.pos[2]).collect();
        z.iter().cloned().fold(f32::MIN, f32::max) - z.iter().cloned().fold(f32::MAX, f32::min)
    }

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
        a.set_clip(Clip::Walk);
        a.advance(FRAC_PI_2 / Clip::Walk.cadence());
        assert!(z_span(&a) > 0.5, "al caminar los miembros adelantan/atrasan: {}", z_span(&a));
    }

    #[test]
    fn idle_casi_quieto() {
        // En Idle los miembros no se balancean: el span en Z es chico.
        let mut a = Actor::new(Vec3::ZERO, 0.0); // Idle por defecto
        a.advance(FRAC_PI_2 / Clip::Idle.cadence());
        assert!(z_span(&a) < 0.45, "Idle no debería balancear: {}", z_span(&a));
    }

    #[test]
    fn cambiar_de_clip_reinicia_la_fase() {
        let mut a = Actor::new(Vec3::ZERO, 0.0);
        a.set_clip(Clip::Walk);
        a.advance(1.0);
        assert!(a.phase > 0.0);
        a.set_clip(Clip::Run);
        assert_eq!(a.phase, 0.0, "un clip nuevo arranca la pose desde 0");
        // Repetir el mismo clip NO corta la fase.
        a.advance(0.5);
        let ph = a.phase;
        a.set_clip(Clip::Run);
        assert_eq!(a.phase, ph);
    }

    #[test]
    fn cambio_de_clip_hace_cross_fade() {
        // Caminando con las piernas bien abiertas…
        let mut a = Actor::new(Vec3::ZERO, 0.0);
        a.set_clip(Clip::Walk);
        a.advance(FRAC_PI_2 / Clip::Walk.cadence());
        let span_walk = z_span(&a);
        assert!(span_walk > 0.5);

        // …al pasar a Idle, JUSTO después la pose sigue siendo ~la de caminar
        // (blend≈0), no salta de golpe a quieto.
        a.set_clip(Clip::Idle);
        let span_inicio = z_span(&a);
        assert!((span_inicio - span_walk).abs() < 0.05, "el cross-fade arranca desde la pose saliente");

        // Pasado el blend, ya es Idle (piernas juntas).
        a.advance(BLEND_DUR + 0.1);
        assert!(z_span(&a) < 0.45, "tras el cross-fade la pose es Idle");
    }

    #[test]
    fn face_towards_mira_a_mas_z() {
        let mut a = Actor::new(Vec3::ZERO, 0.0);
        a.face_towards(Vec3::new(0.0, 0.0, 5.0));
        assert!(a.facing.abs() < 1e-4, "mirar a +Z → yaw≈0");
    }
}
