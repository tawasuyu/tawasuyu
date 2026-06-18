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
use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    /// `true` si el clip es un **gesto** (no locomoción) — un momento expresivo que
    /// merece un acento musical. Lo usa el director para derivar los "beats del guion".
    pub fn is_emote(self) -> bool {
        matches!(self, Clip::Wave | Clip::Point | Clip::Cheer)
    }

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

/// **Edad cuantizada** del personaje: estadios discretos que cambian las
/// proporciones del cuerpo (un bebé es cabezón y de miembros cortos; un adulto es
/// alto y proporcionado). Sirve para *mostrar al niño primero* (el corto: nace en el
/// desierto) y envejecerlo por etapas. Cada edad deriva un [`Build`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Age {
    /// Bebé/recién nacido: chiquito, cabezón, miembros cortos.
    Baby,
    /// Niño.
    Child,
    /// Joven/adolescente.
    Teen,
    /// Adulto (proporciones de referencia).
    Adult,
    /// Anciano (apenas más bajo que el adulto).
    Elder,
}

impl Age {
    /// Todas las edades, de menor a mayor (para que un editor cicle entre ellas).
    pub const ALL: [Age; 5] = [Age::Baby, Age::Child, Age::Teen, Age::Adult, Age::Elder];

    /// Nombre legible (español) para la UI.
    pub fn label(self) -> &'static str {
        match self {
            Age::Baby => "bebé",
            Age::Child => "niño",
            Age::Teen => "joven",
            Age::Adult => "adulto",
            Age::Elder => "anciano",
        }
    }

    /// La edad siguiente (cicla) — para botones de ciclo.
    pub fn next(self) -> Age {
        let i = Age::ALL.iter().position(|&a| a == self).unwrap_or(0);
        Age::ALL[(i + 1) % Age::ALL.len()]
    }

    /// `(escala_total, refuerzo_cabeza, escala_miembros)` por edad. Más joven =
    /// más chico, cabeza proporcionalmente más grande y miembros más cortos.
    fn params(self) -> (f32, f32, f32) {
        match self {
            Age::Baby => (0.50, 1.55, 0.70),
            Age::Child => (0.66, 1.28, 0.82),
            Age::Teen => (0.84, 1.08, 0.93),
            Age::Adult => (1.00, 1.00, 1.00),
            Age::Elder => (0.96, 1.00, 0.97),
        }
    }
}

/// **Constitución** del muñeco: las medidas de cada parte (posiciones de
/// articulación y tamaños de caja), en el espacio local del actor (pies en el
/// origen, mirando a `+Z`). Es el "esqueleto + modelado" configurable: cambiarla
/// hace personajes distintos (alto/bajo, cabezón, etc.). Se construye por
/// [`Age`](Age) ([`Build::for_age`]) y los pies quedan **siempre en `y=0`** por
/// construcción (`hip_y == leg_len`).
#[derive(Debug, Clone, Copy)]
pub struct Build {
    /// Altura aproximada total (referencia).
    pub height: f32,
    /// Centro y tamaño del torso.
    pub torso_y: f32,
    pub torso: Vec3,
    /// Centro y tamaño de la cabeza.
    pub head_y: f32,
    pub head: Vec3,
    /// Altura del hombro y separación lateral; largo y tamaño del brazo.
    pub shoulder_y: f32,
    pub shoulder_x: f32,
    pub arm_len: f32,
    pub arm: Vec3,
    /// Altura de la cadera (= `leg_len`, pies en el piso), separación; largo/tamaño
    /// de la pierna.
    pub hip_y: f32,
    pub hip_x: f32,
    pub leg_len: f32,
    pub leg: Vec3,
    /// Tamaño de la mano.
    pub hand: Vec3,
}

impl Build {
    /// Construye la constitución de una [`Age`]. Bottom-up desde los pies (`y=0`),
    /// así cualquier edad queda con los pies en el piso. `Adult` reproduce las
    /// proporciones históricas del muñeco (los 11 cubos de siempre).
    pub fn for_age(age: Age) -> Build {
        let (s, head_boost, limb) = age.params();
        let neck = 0.02 * s;
        let leg_len = 0.80 * s * limb;
        let hip_y = leg_len; // pies en el piso
        let torso = Vec3::new(0.55 * s, 0.60 * s, 0.30 * s);
        let torso_y = hip_y + torso.y * 0.5;
        let shoulder_y = hip_y + torso.y;
        let head = Vec3::new(0.42 * s * head_boost, 0.40 * s * head_boost, 0.42 * s * head_boost);
        let head_y = shoulder_y + head.y * 0.5 + neck;
        Build {
            height: head_y + head.y * 0.5,
            torso_y,
            torso,
            head_y,
            head,
            shoulder_y,
            shoulder_x: 0.36 * s,
            arm_len: 0.60 * s * limb,
            arm: Vec3::new(0.18 * s, 0.60 * s * limb, 0.18 * s),
            hip_y,
            hip_x: 0.14 * s,
            leg_len,
            leg: Vec3::new(0.22 * s, leg_len, 0.22 * s),
            hand: Vec3::new(0.20 * s, 0.18 * s, 0.20 * s),
        }
    }

    /// Constitución adulta de referencia.
    pub fn adult() -> Build {
        Build::for_age(Age::Adult)
    }
}

/// Personaje articulado. `pos` es el **centro de los pies** en espacio de mundo
/// (las mismas coordenadas del terreno/grid); `facing` el rumbo (yaw, `0`=`+Z`).
/// `clip`/`phase` definen la animación actual. Colores por zona (piel/remera/
/// pantalón). La [`Build`] define las proporciones (edad/personaje).
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
    /// **IK de mirada** (look-at constraint): si está, la cabeza gira (yaw+pitch,
    /// dentro de un rango creíble) para **mirar ese punto de mundo**, por encima del
    /// cabeceo del clip — los ojos siguen al objetivo. `None` = cabeza alineada al
    /// cuerpo. La fija [`look_at`](Self::look_at).
    look_target: Option<Vec3>,
    /// Constitución (proporciones por edad/personaje). La fija
    /// [`with_age`](Self::with_age) / [`with_build`](Self::with_build).
    pub build: Build,
    /// Edad actual (estadio cuantizado) — informativo; el cuerpo lo da `build`.
    pub age: Age,
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
            look_target: None,
            build: Build::adult(),
            age: Age::Adult,
        }
    }

    /// Fija la **edad** (estadio cuantizado) → recalcula la constitución del cuerpo.
    /// Encadenable: `Actor::new(pos, yaw).with_age(Age::Baby)`. Para *mostrar al
    /// niño primero* y envejecerlo por etapas.
    pub fn with_age(mut self, age: Age) -> Self {
        self.set_age(age);
        self
    }

    /// Cambia la edad en caliente (recalcula `build`).
    pub fn set_age(&mut self, age: Age) {
        self.age = age;
        self.build = Build::for_age(age);
    }

    /// Fija una constitución arbitraria (personaje a medida, no atado a una edad).
    pub fn with_build(mut self, build: Build) -> Self {
        self.build = build;
        self
    }

    /// Fija (o limpia con `None`) el **objetivo de mirada** (IK de cabeza): la cabeza
    /// y los ojos se orientan hacia ese punto de mundo, dentro de un rango creíble,
    /// sin mover el cuerpo. Útil para que un actor "mire a cámara" o siga algo.
    pub fn look_at(&mut self, target: Option<Vec3>) {
        self.look_target = target;
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
        let b = &self.build;
        let mut v = Vec::with_capacity(8 * 11);
        let mut i = Vec::with_capacity(36 * 11);

        // Transform del cuerpo superior: rebote vertical + inclinación adelante
        // (rotación en X alrededor de los pies/origen).
        let body = Mat4::from_translation(Vec3::new(0.0, p.bob, 0.0)) * Mat4::from_rotation_x(p.lean);

        // Torso.
        push_cube(&mut v, &mut i, body * trs(Vec3::new(0.0, b.torso_y, 0.0), Mat4::IDENTITY, b.torso), self.shirt);

        // Cabeza: cabeceo del clip + IK de mirada (yaw/pitch hacia el objetivo). El
        // `head_anchor` (sin escala) ancla cabeza, ojos y boca para que giren juntos.
        let (look_yaw, look_pitch) = self.look_angles();
        let head_rot = Mat4::from_rotation_y(look_yaw) * Mat4::from_rotation_x(p.head_pitch + look_pitch);
        let head_anchor = body * Mat4::from_translation(Vec3::new(0.0, b.head_y, 0.0)) * head_rot;
        push_cube(&mut v, &mut i, head_anchor * Mat4::from_scale(b.head), self.skin);

        // Cara: dos ojos + boca en la cara `+Z` de la cabeza. Las posiciones/tamaños
        // van como **fracción del tamaño de la cabeza** → escalan con la edad (un bebé
        // cabezón tiene ojos más grandes). Decales finos (apenas sobresalen, estilo
        // Minecraft). Parpadeo determinista + boca que se abre con los gestos.
        let (hw, hh, hd) = (b.head.x, b.head.y, b.head.z);
        let blink = self.blink(); // 1 = abierto, ~0 = cerrado
        let eye_sz = Vec3::new(0.095 * hw, (0.15 * hh * blink).max(0.012), 0.05 * hd);
        let face_z = hd * 0.49; // casi en la cara +Z
        for sx in [0.26_f32, -0.26] {
            push_cube(
                &mut v,
                &mut i,
                head_anchor * trs(Vec3::new(sx * hw, 0.12 * hh, face_z), Mat4::IDENTITY, eye_sz),
                EYE_COLOR,
            );
        }
        let mouth_open = self.mouth_open();
        push_cube(
            &mut v,
            &mut i,
            head_anchor * trs(Vec3::new(0.0, -0.25 * hh, face_z), Mat4::IDENTITY, Vec3::new(0.38 * hw, 0.05 * hh + mouth_open, 0.05 * hd)),
            MOUTH_COLOR,
        );

        // Piernas (sin `body`: pies plantados). Articulación en la cadera.
        let leg_rot_r = Mat4::from_rotation_x(p.leg_r);
        let leg_rot_l = Mat4::from_rotation_x(p.leg_l);
        limb(&mut v, &mut i, Mat4::IDENTITY, Vec3::new(b.hip_x, b.hip_y, 0.0), b.leg_len, b.leg, leg_rot_r, self.pants);
        limb(&mut v, &mut i, Mat4::IDENTITY, Vec3::new(-b.hip_x, b.hip_y, 0.0), b.leg_len, b.leg, leg_rot_l, self.pants);

        // Brazos (con `body`). Rotación = apertura(Z)·balanceo(X); apertura espejada.
        let arm_r_rot = Mat4::from_rotation_z(p.arm_r_out) * Mat4::from_rotation_x(p.arm_r);
        let arm_l_rot = Mat4::from_rotation_z(-p.arm_l_out) * Mat4::from_rotation_x(p.arm_l);
        let (sh_r, sh_l) = (Vec3::new(b.shoulder_x, b.shoulder_y, 0.0), Vec3::new(-b.shoulder_x, b.shoulder_y, 0.0));
        limb(&mut v, &mut i, body, sh_r, b.arm_len, b.arm, arm_r_rot, self.shirt);
        limb(&mut v, &mut i, body, sh_l, b.arm_len, b.arm, arm_l_rot, self.shirt);

        // Manos: una caja de piel en la punta de cada brazo (a `arm_len` del hombro).
        hand_at(&mut v, &mut i, body, sh_r, b.arm_len, arm_r_rot, b.hand, self.skin);
        hand_at(&mut v, &mut i, body, sh_l, b.arm_len, arm_l_rot, b.hand, self.skin);

        (v, i)
    }

    /// Ángulos `(yaw, pitch)` de la cabeza para el IK de mirada: dirección al objetivo
    /// llevada al espacio local del actor (deshaciendo `facing`) y acotada a un rango
    /// creíble (±70° yaw, ±50° pitch) para que el cuello no se quiebre. `(0,0)` si no
    /// hay objetivo.
    fn look_angles(&self) -> (f32, f32) {
        use std::f32::consts::FRAC_PI_2;
        let Some(target) = self.look_target else { return (0.0, 0.0) };
        // Posición aproximada de la cabeza en mundo (pies + 1.62 de altura).
        let head_pos = self.pos + Vec3::new(0.0, 1.62, 0.0);
        let d = target - head_pos;
        if d.length_squared() < 1e-6 {
            return (0.0, 0.0);
        }
        // A espacio local: deshacer el yaw del cuerpo.
        let local = Mat4::from_rotation_y(-self.facing).transform_vector3(d).normalize();
        let yaw = local.x.atan2(local.z).clamp(-1.22, 1.22); // ±70°
        let pitch = (-local.y).asin().clamp(-0.87, 0.87); // ±50°, mirar arriba/abajo
        let _ = FRAC_PI_2;
        (yaw, pitch)
    }

    /// Apertura de párpados `0..1` (1 = abierto). Parpadeo determinista por la fase:
    /// un cierre breve cada ~3 unidades de fase. Idle no necesita objetivo.
    fn blink(&self) -> f32 {
        let ph = self.phase * 0.33; // ciclos lentos
        let f = ph - ph.floor();
        // Cerrado sólo en una ventana corta (~6% del ciclo).
        if f > 0.94 {
            // Sube y baja rápido dentro de la ventana (triángulo invertido).
            let w = (f - 0.94) / 0.06; // 0..1
            (1.0 - (1.0 - (2.0 * w - 1.0).abs())).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    /// Apertura de la boca (unidades): abierta en los gestos expresivos (festejar más
    /// que saludar/señalar), cerrada el resto → la cara "reacciona" al clip.
    fn mouth_open(&self) -> f32 {
        match self.clip {
            Clip::Cheer => 0.10 + 0.04 * self.phase.sin().abs(),
            Clip::Wave | Clip::Point => 0.05,
            _ => 0.0,
        }
    }
}

/// Color de los ojos (casi negro).
const EYE_COLOR: [f32; 3] = [0.08, 0.07, 0.09];
/// Color de la boca (marrón oscuro).
const MOUTH_COLOR: [f32; 3] = [0.30, 0.12, 0.12];

/// Apila la **mano** en la punta de un miembro: una caja de tamaño `hand` centrada a
/// `len` del pivote `joint` (donde termina la caja del brazo), con la misma rotación
/// `rot` del brazo y el mismo prefijo `pre`.
#[allow(clippy::too_many_arguments)]
fn hand_at(
    v: &mut Vec<Vertex3d>,
    i: &mut Vec<u16>,
    pre: Mat4,
    joint: Vec3,
    len: f32,
    rot: Mat4,
    hand: Vec3,
    color: [f32; 3],
) {
    let m = pre
        * Mat4::from_translation(joint)
        * rot
        * Mat4::from_translation(Vec3::new(0.0, -len, 0.0))
        * Mat4::from_scale(hand);
    push_cube(v, i, m, color);
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
    fn malla_tiene_once_cajas() {
        // 6 del cuerpo (torso/cabeza/2 piernas/2 brazos) + 2 manos + 2 ojos + boca.
        let a = Actor::new(Vec3::ZERO, 0.0);
        let (v, idx) = a.mesh();
        assert_eq!(v.len(), 8 * 11, "11 cajas × 8 vértices");
        assert_eq!(idx.len(), 36 * 11, "11 cajas × 36 índices");
    }

    #[test]
    fn edades_cambian_proporciones_y_dejan_los_pies_en_el_piso() {
        let baby = Build::for_age(Age::Baby);
        let adult = Build::for_age(Age::Adult);
        // El bebé es más bajo que el adulto.
        assert!(baby.height < adult.height, "bebé {} < adulto {}", baby.height, adult.height);
        // ...y CABEZÓN: la cabeza ocupa una fracción mayor de su altura.
        let head_frac = |b: &Build| b.head.y / b.height;
        assert!(head_frac(&baby) > head_frac(&adult) + 0.05, "bebé cabezón: {} vs {}", head_frac(&baby), head_frac(&adult));
        // Toda edad apoya los pies en y=0 (el voxel más bajo de la malla ≈ 0).
        for age in [Age::Baby, Age::Child, Age::Teen, Age::Adult, Age::Elder] {
            let a = Actor::new(Vec3::ZERO, 0.0).with_age(age);
            let ymin = a.mesh().0.iter().map(|v| v.pos[1]).fold(f32::MAX, f32::min);
            assert!(ymin.abs() < 1e-3, "pies en el piso para {age:?}: ymin={ymin}");
        }
    }

    #[test]
    fn adulto_conserva_los_once_cubos_y_altura_historica() {
        // El refactor a Build no cambió al adulto: 11 cajas y altura ~1.82.
        let a = Actor::new(Vec3::ZERO, 0.0);
        assert_eq!(a.mesh().0.len(), 8 * 11);
        assert!((a.build.height - 1.82).abs() < 0.05, "altura adulta {}", a.build.height);
    }

    #[test]
    fn ik_de_mirada_gira_la_cabeza_hacia_el_objetivo() {
        // Mirar a la derecha (mundo +X) vs a la izquierda (−X): los ojos (vértices más
        // adelantados en la cara) deben desplazarse en X en sentidos opuestos.
        let eye_centroid_x = |a: &Actor| {
            // Los ojos están en la cara +Z, son los vértices con mayor z y |x|≈0.11.
            let verts = a.mesh().0;
            let zmax = verts.iter().map(|v| v.pos[2]).fold(f32::MIN, f32::max);
            let front: Vec<f32> =
                verts.iter().filter(|v| v.pos[2] > zmax - 0.05).map(|v| v.pos[0]).collect();
            front.iter().sum::<f32>() / front.len().max(1) as f32
        };
        let mut right = Actor::new(Vec3::ZERO, 0.0);
        right.look_at(Some(Vec3::new(10.0, 1.62, 1.0)));
        let mut left = Actor::new(Vec3::ZERO, 0.0);
        left.look_at(Some(Vec3::new(-10.0, 1.62, 1.0)));
        // Mirar a +X adelanta la cara hacia +X; a −X, hacia −X.
        assert!(eye_centroid_x(&right) > eye_centroid_x(&left) + 0.05, "la cabeza sigue al objetivo");
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
