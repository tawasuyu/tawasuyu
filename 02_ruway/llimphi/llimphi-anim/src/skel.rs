//! Jerarquía de huesos 2D + skinning ponderado de vértices (linear blend
//! skinning, LBS) — el sustrato de la deformación esqueletal estilo Rive.
//!
//! **Puro y sin renderer.** Este módulo sólo calcula geometría: dada una
//! jerarquía de huesos posada y una malla con pesos por vértice, produce las
//! **posiciones deformadas** de los vértices (`kurbo::Point`). Quién las pinta
//! (vello vía paths para malla vectorial, o clip+imagen para malla texturizada)
//! es trabajo del consumidor; el spike confirmó que ambas rutas funcionan en
//! vello 0.7.
//!
//! ## Bind pose vs pose actual
//!
//! El esqueleto se construye en su **bind pose** (la configuración en la que se
//! ató la malla) y se llama [`Skeleton::bind`] para congelarla: por cada hueso
//! se guarda su *inverse-bind* (el world transform inverso en el bind). Al
//! animar, se cambian las poses locales ([`Skeleton::set_pose`]),
//! [`Skeleton::update`] recompone los world transforms, y la matriz de skinning
//! de un hueso es `world_actual · inverse_bind`. En el bind pose esa matriz es
//! la identidad → los vértices quedan en reposo.
//!
//! ## LBS
//!
//! La posición deformada de un vértice es la media ponderada, sobre los huesos
//! que lo influyen, de aplicarle a su posición de reposo la matriz de skinning
//! de cada hueso: `v' = Σ_b w_b · (skin_b · v_rest)`.

use std::collections::HashMap;

use kurbo::{Affine, Point, Vec2};

use crate::RenderFrame;

/// Índice de un hueso dentro del esqueleto.
pub type BoneId = usize;

/// Pose local de un hueso (TRS) relativa a su padre. Se compone como
/// `translate · rotate · scale` — el orden estándar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub translation: Vec2,
    /// Rotación en radianes (convención de kurbo).
    pub rotation: f64,
    pub scale: Vec2,
}

impl Default for Pose {
    fn default() -> Self {
        Self::identity()
    }
}

impl Pose {
    pub fn identity() -> Self {
        Self {
            translation: Vec2::ZERO,
            rotation: 0.0,
            scale: Vec2::new(1.0, 1.0),
        }
    }
    pub fn translate(t: Vec2) -> Self {
        Self {
            translation: t,
            ..Self::identity()
        }
    }
    pub fn rotate(r: f64) -> Self {
        Self {
            rotation: r,
            ..Self::identity()
        }
    }
    /// Translación + rotación (lo más común al animar un hueso).
    pub fn new(translation: Vec2, rotation: f64, scale: Vec2) -> Self {
        Self {
            translation,
            rotation,
            scale,
        }
    }
    pub fn to_affine(&self) -> Affine {
        Affine::translate(self.translation)
            * Affine::rotate(self.rotation)
            * Affine::scale_non_uniform(self.scale.x, self.scale.y)
    }
}

#[derive(Debug, Clone)]
struct Bone {
    parent: Option<BoneId>,
    local: Pose,
}

/// Una jerarquía de huesos posable. Los huesos se agregan **padre antes que
/// hijo** (el índice del padre debe ser menor), así el cómputo de world
/// transforms es una sola pasada hacia adelante.
#[derive(Debug, Clone, Default)]
pub struct Skeleton {
    bones: Vec<Bone>,
    /// World transform inverso de cada hueso, capturado en `bind`.
    inverse_bind: Vec<Affine>,
    /// Scratch: world transforms actuales (recomputados por `update`).
    world: Vec<Affine>,
}

impl Skeleton {
    pub fn new() -> Self {
        Self::default()
    }

    /// Agrega un hueso con `parent` (o `None` = raíz) y su pose local. Devuelve
    /// su `BoneId`. **Pánico** si el padre no fue agregado antes (índice ≥ id).
    pub fn add_bone(&mut self, parent: Option<BoneId>, local: Pose) -> BoneId {
        if let Some(p) = parent {
            assert!(
                p < self.bones.len(),
                "el padre {p} debe agregarse antes que el hijo"
            );
        }
        let id = self.bones.len();
        self.bones.push(Bone { parent, local });
        self.inverse_bind.push(Affine::IDENTITY);
        self.world.push(Affine::IDENTITY);
        id
    }

    fn recompute_world(&mut self) {
        for i in 0..self.bones.len() {
            let local = self.bones[i].local.to_affine();
            self.world[i] = match self.bones[i].parent {
                Some(p) => self.world[p] * local,
                None => local,
            };
        }
    }

    /// Congela la pose actual como **bind pose**: recompone los world transforms
    /// y guarda el inverse-bind de cada hueso. Llamar una vez tras construir el
    /// esqueleto en reposo (antes de animar).
    pub fn bind(&mut self) {
        self.recompute_world();
        for i in 0..self.bones.len() {
            self.inverse_bind[i] = self.world[i].inverse();
        }
    }

    /// Cambia la pose local de un hueso (animar). Requiere [`update`] después
    /// para que los world transforms reflejen el cambio.
    ///
    /// [`update`]: Skeleton::update
    pub fn set_pose(&mut self, bone: BoneId, local: Pose) {
        self.bones[bone].local = local;
    }

    /// Pose local actual de un hueso.
    pub fn pose(&self, bone: BoneId) -> Pose {
        self.bones[bone].local
    }

    /// Recalcula los world transforms desde las poses actuales. Llamar tras
    /// posear y antes de deformar/leer `world`/`skin_matrix`.
    pub fn update(&mut self) {
        self.recompute_world();
    }

    /// Matriz de skinning del hueso: `world_actual · inverse_bind`. En el bind
    /// pose es la identidad.
    pub fn skin_matrix(&self, bone: BoneId) -> Affine {
        self.world[bone] * self.inverse_bind[bone]
    }

    /// World transform actual del hueso (útil para dibujar el hueso mismo o
    /// adjuntar algo a su punta).
    pub fn world(&self, bone: BoneId) -> Affine {
        self.world[bone]
    }

    pub fn len(&self) -> usize {
        self.bones.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bones.is_empty()
    }
}

/// Influencia de un hueso sobre un vértice. Idealmente los pesos de un vértice
/// suman 1; si no, `deform` normaliza.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weight {
    pub bone: BoneId,
    pub weight: f64,
}

/// Un vértice de la malla: su posición de reposo (en model/bind space), su UV
/// `0..1` (para textura, lo usa el render de malla texturizada) y los huesos que
/// lo influyen.
#[derive(Debug, Clone)]
pub struct Vertex {
    pub rest: Point,
    pub uv: (f64, f64),
    pub weights: Vec<Weight>,
}

impl Vertex {
    /// Vértice rígido a un solo hueso (peso 1).
    pub fn rigid(rest: Point, uv: (f64, f64), bone: BoneId) -> Self {
        Self {
            rest,
            uv,
            weights: vec![Weight { bone, weight: 1.0 }],
        }
    }
}

/// Una malla deformable: vértices con pesos + topología de triángulos (índices
/// a `vertices`). El render la consume tras `deform`.
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub triangles: Vec<[u32; 3]>,
}

impl Mesh {
    pub fn new() -> Self {
        Self::default()
    }

    /// Calcula las posiciones deformadas (LBS) en `out`, una por vértice, en el
    /// mismo orden que `self.vertices`. `skel` debe tener sus world transforms
    /// actualizados ([`Skeleton::update`]). Reutiliza el buffer `out` (sin
    /// asignar por frame).
    pub fn deform_into(&self, skel: &Skeleton, out: &mut Vec<Point>) {
        out.clear();
        out.reserve(self.vertices.len());
        for v in &self.vertices {
            if v.weights.is_empty() {
                out.push(v.rest);
                continue;
            }
            let mut acc = Vec2::ZERO;
            let mut wsum = 0.0;
            for w in &v.weights {
                let p = skel.skin_matrix(w.bone) * v.rest;
                acc += p.to_vec2() * w.weight;
                wsum += w.weight;
            }
            let p = if wsum.abs() > 1e-12 {
                (acc * (1.0 / wsum)).to_point()
            } else {
                v.rest
            };
            out.push(p);
        }
    }

    /// Variante que asigna y devuelve el `Vec` (conveniencia para tests/uso
    /// ocasional; en el bucle de render preferí [`deform_into`]).
    ///
    /// [`deform_into`]: Mesh::deform_into
    pub fn deform(&self, skel: &Skeleton) -> Vec<Point> {
        let mut out = Vec::new();
        self.deform_into(skel, &mut out);
        out
    }
}

/// Interpola linealmente dos poses (translación/escala lerp, rotación lerp del
/// ángulo). Es el blend de poses que usa el crossfade esqueletal — mezclar
/// *poses* da una deformación correcta, a diferencia del crossfade de píxeles.
pub fn lerp_pose(a: Pose, b: Pose, f: f64) -> Pose {
    let lerp = |x: f64, y: f64| x + (y - x) * f;
    Pose {
        translation: Vec2::new(
            lerp(a.translation.x, b.translation.x),
            lerp(a.translation.y, b.translation.y),
        ),
        rotation: lerp(a.rotation, b.rotation),
        scale: Vec2::new(lerp(a.scale.x, b.scale.x), lerp(a.scale.y, b.scale.y)),
    }
}

/// Keyframe de pose de un hueso en un instante.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoseKey {
    pub t: f64,
    pub pose: Pose,
}

/// Pista de animación de un hueso: keyframes de pose ordenados por tiempo.
#[derive(Debug, Clone, Default)]
pub struct BoneTrack {
    pub bone: BoneId,
    pub keys: Vec<PoseKey>,
}

impl BoneTrack {
    /// Pose en el instante `t` (lineal entre keyframes, clamp en los extremos).
    pub fn sample(&self, t: f64) -> Pose {
        match self.keys.as_slice() {
            [] => Pose::identity(),
            [only] => only.pose,
            keys => {
                if t <= keys[0].t {
                    return keys[0].pose;
                }
                let last = keys[keys.len() - 1];
                if t >= last.t {
                    return last.pose;
                }
                for w in keys.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    if t >= a.t && t <= b.t {
                        let span = b.t - a.t;
                        let f = if span > 1e-12 { (t - a.t) / span } else { 0.0 };
                        return lerp_pose(a.pose, b.pose, f);
                    }
                }
                last.pose
            }
        }
    }
}

/// Una animación esqueletal: pistas de pose por hueso, con duración y loop. Es
/// lo que un *estado* de la máquina reproduce (un clip = una `BoneAnimation`).
/// Los huesos no incluidos en ninguna pista conservan su pose actual.
#[derive(Debug, Clone, Default)]
pub struct BoneAnimation {
    pub duration: f64,
    pub looping: bool,
    pub tracks: Vec<BoneTrack>,
}

impl BoneAnimation {
    /// Samplea todas las pistas en `t` (envuelto por la duración si es loop) a
    /// pares `(hueso, pose)`, reusando el buffer `out`.
    pub fn sample_into(&self, t: f64, out: &mut Vec<(BoneId, Pose)>) {
        let tt = if self.looping && self.duration > 0.0 {
            t.rem_euclid(self.duration)
        } else {
            t
        };
        out.clear();
        out.extend(self.tracks.iter().map(|tr| (tr.bone, tr.sample(tt))));
    }

    /// Posa el esqueleto con esta animación en el instante `t` y llama `update`.
    pub fn apply(&self, skel: &mut Skeleton, t: f64) {
        let mut buf = Vec::new();
        self.sample_into(t, &mut buf);
        for (b, p) in buf {
            skel.set_pose(b, p);
        }
        skel.update();
    }
}

/// Posa el esqueleto según el [`RenderFrame`] de una máquina de estados, usando
/// las `clips` ([`BoneAnimation`]) indexadas por `ClipId`. Durante una
/// transición blendea **poses** (no píxeles) por la mezcla del frame. Llama
/// `skel.update()` al final. Es el puente que une máquina-de-estados →
/// esqueleto: estado/transición deciden la pose, el skinning la convierte en
/// deformación.
///
/// [`RenderFrame`]: crate::RenderFrame
pub fn pose_from_render_frame(skel: &mut Skeleton, frame: &RenderFrame, clips: &[BoneAnimation]) {
    let mut poses: HashMap<BoneId, Pose> = HashMap::new();
    let mut buf = Vec::new();
    if let Some(a) = clips.get(frame.primary.clip as usize) {
        a.sample_into(frame.primary.time_secs, &mut buf);
        for (b, p) in buf.drain(..) {
            poses.insert(b, p);
        }
    }
    if let Some((inc, mix)) = frame.blend {
        if let Some(a) = clips.get(inc.clip as usize) {
            a.sample_into(inc.time_secs, &mut buf);
            let mix = mix as f64;
            for (b, p) in buf.drain(..) {
                poses
                    .entry(b)
                    .and_modify(|cur| *cur = lerp_pose(*cur, p, mix))
                    .or_insert(p);
            }
        }
    }
    for (b, p) in poses {
        skel.set_pose(b, p);
    }
    skel.update();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Point, x: f64, y: f64) -> bool {
        (a.x - x).abs() < 1e-9 && (a.y - y).abs() < 1e-9
    }

    #[test]
    fn bind_pose_deja_los_vertices_en_reposo() {
        let mut s = Skeleton::new();
        let b = s.add_bone(None, Pose::translate(Vec2::new(5.0, 7.0)));
        s.bind();
        s.update();
        let mut m = Mesh::new();
        m.vertices.push(Vertex::rigid(Point::new(3.0, 4.0), (0.0, 0.0), b));
        let d = m.deform(&s);
        // Sin re-posar: la matriz de skinning es identidad → reposo intacto.
        assert!(approx(d[0], 3.0, 4.0), "fue {:?}", d[0]);
    }

    #[test]
    fn rotar_un_hueso_rota_su_vertice() {
        let mut s = Skeleton::new();
        let b = s.add_bone(None, Pose::identity());
        s.bind();
        // Rotar 90°: (10,0) → (0,10) en convención de kurbo.
        s.set_pose(b, Pose::rotate(std::f64::consts::FRAC_PI_2));
        s.update();
        let mut m = Mesh::new();
        m.vertices.push(Vertex::rigid(Point::new(10.0, 0.0), (0.0, 0.0), b));
        let d = m.deform(&s);
        assert!(approx(d[0], 0.0, 10.0), "fue {:?}", d[0]);
    }

    #[test]
    fn cadena_padre_hijo_compone_transforms() {
        let mut s = Skeleton::new();
        let root = s.add_bone(None, Pose::identity());
        let child = s.add_bone(Some(root), Pose::translate(Vec2::new(10.0, 0.0)));
        s.bind();
        // Rotar la raíz 90° rota rígidamente al hijo y su vértice.
        s.set_pose(root, Pose::rotate(std::f64::consts::FRAC_PI_2));
        s.update();
        let mut m = Mesh::new();
        m.vertices
            .push(Vertex::rigid(Point::new(20.0, 0.0), (0.0, 0.0), child));
        let d = m.deform(&s);
        // (20,0) rotado 90° sobre el origen → (0,20).
        assert!(approx(d[0], 0.0, 20.0), "fue {:?}", d[0]);
    }

    #[test]
    fn peso_repartido_mezcla_dos_huesos() {
        let mut s = Skeleton::new();
        let a = s.add_bone(None, Pose::identity());
        let b = s.add_bone(None, Pose::identity());
        s.bind();
        // A queda quieto; B se traslada (0,20).
        s.set_pose(b, Pose::translate(Vec2::new(0.0, 20.0)));
        s.update();
        let mut m = Mesh::new();
        m.vertices.push(Vertex {
            rest: Point::new(0.0, 0.0),
            uv: (0.5, 0.5),
            weights: vec![
                Weight { bone: a, weight: 0.5 },
                Weight { bone: b, weight: 0.5 },
            ],
        });
        let d = m.deform(&s);
        // 0.5·(0,0) + 0.5·(0,20) = (0,10).
        assert!(approx(d[0], 0.0, 10.0), "fue {:?}", d[0]);
    }

    #[test]
    fn pesos_sin_normalizar_se_normalizan() {
        let mut s = Skeleton::new();
        let a = s.add_bone(None, Pose::identity());
        let b = s.add_bone(None, Pose::translate(Vec2::new(0.0, 10.0)));
        s.bind();
        s.update();
        let mut m = Mesh::new();
        // Pesos 2 y 2 (suman 4) → media ponderada, no suma cruda.
        m.vertices.push(Vertex {
            rest: Point::new(0.0, 0.0),
            uv: (0.0, 0.0),
            weights: vec![
                Weight { bone: a, weight: 2.0 },
                Weight { bone: b, weight: 2.0 },
            ],
        });
        let d = m.deform(&s);
        // skin_a = id → (0,0); skin_b = id (b en bind pose, no re-posado) → (0,0).
        // Ambos en reposo: (0,0). (Verifica que no explota por wsum=4.)
        assert!(approx(d[0], 0.0, 0.0), "fue {:?}", d[0]);
    }

    #[test]
    fn vertice_sin_pesos_queda_en_reposo() {
        let mut s = Skeleton::new();
        let _b = s.add_bone(None, Pose::rotate(1.0));
        s.bind();
        s.update();
        let mut m = Mesh::new();
        m.vertices.push(Vertex {
            rest: Point::new(7.0, 7.0),
            uv: (0.0, 0.0),
            weights: vec![],
        });
        let d = m.deform(&s);
        assert!(approx(d[0], 7.0, 7.0), "fue {:?}", d[0]);
    }

    #[test]
    fn track_samplea_lineal_entre_keyframes() {
        let tr = BoneTrack {
            bone: 0,
            keys: vec![
                PoseKey { t: 0.0, pose: Pose::rotate(0.0) },
                PoseKey { t: 2.0, pose: Pose::rotate(2.0) },
            ],
        };
        assert!((tr.sample(0.0).rotation - 0.0).abs() < 1e-9);
        assert!((tr.sample(2.0).rotation - 2.0).abs() < 1e-9);
        assert!((tr.sample(1.0).rotation - 1.0).abs() < 1e-9, "punto medio");
        // Clamp fuera de rango.
        assert!((tr.sample(-5.0).rotation - 0.0).abs() < 1e-9);
        assert!((tr.sample(99.0).rotation - 2.0).abs() < 1e-9);
    }

    #[test]
    fn animacion_en_loop_envuelve_el_tiempo() {
        let anim = BoneAnimation {
            duration: 2.0,
            looping: true,
            tracks: vec![BoneTrack {
                bone: 0,
                keys: vec![
                    PoseKey { t: 0.0, pose: Pose::rotate(0.0) },
                    PoseKey { t: 2.0, pose: Pose::rotate(2.0) },
                ],
            }],
        };
        let mut buf = Vec::new();
        anim.sample_into(3.0, &mut buf); // 3 % 2 = 1 → rotación ~1.0
        assert!((buf[0].1.rotation - 1.0).abs() < 1e-9, "fue {}", buf[0].1.rotation);
    }

    #[test]
    fn render_frame_simple_posa_el_esqueleto() {
        let mut s = Skeleton::new();
        let b = s.add_bone(None, Pose::identity());
        s.bind();
        let anim = BoneAnimation {
            duration: 2.0,
            looping: true,
            tracks: vec![BoneTrack {
                bone: b,
                keys: vec![
                    PoseKey { t: 0.0, pose: Pose::identity() },
                    PoseKey {
                        t: 1.0,
                        pose: Pose::rotate(std::f64::consts::FRAC_PI_2),
                    },
                ],
            }],
        };
        let clips = vec![anim];
        let frame = crate::RenderFrame {
            primary: crate::ClipSample { clip: 0, time_secs: 1.0 },
            blend: None,
        };
        pose_from_render_frame(&mut s, &frame, &clips);
        let mut m = Mesh::new();
        m.vertices.push(Vertex::rigid(Point::new(10.0, 0.0), (0.0, 0.0), b));
        let d = m.deform(&s);
        assert!(approx(d[0], 0.0, 10.0), "fue {:?}", d[0]);
    }

    #[test]
    fn render_frame_con_blend_mezcla_poses() {
        let mut s = Skeleton::new();
        let b = s.add_bone(None, Pose::identity());
        s.bind();
        // clip0: identidad; clip1: 90°. Blend 0.5 → 45°.
        let rest = BoneAnimation {
            duration: 1.0,
            looping: true,
            tracks: vec![BoneTrack {
                bone: b,
                keys: vec![PoseKey { t: 0.0, pose: Pose::identity() }],
            }],
        };
        let turn = BoneAnimation {
            duration: 1.0,
            looping: true,
            tracks: vec![BoneTrack {
                bone: b,
                keys: vec![PoseKey {
                    t: 0.0,
                    pose: Pose::rotate(std::f64::consts::FRAC_PI_2),
                }],
            }],
        };
        let clips = vec![rest, turn];
        let frame = crate::RenderFrame {
            primary: crate::ClipSample { clip: 0, time_secs: 0.0 },
            blend: Some((crate::ClipSample { clip: 1, time_secs: 0.0 }, 0.5)),
        };
        pose_from_render_frame(&mut s, &frame, &clips);
        let mut m = Mesh::new();
        m.vertices.push(Vertex::rigid(Point::new(10.0, 0.0), (0.0, 0.0), b));
        let d = m.deform(&s);
        // 10·(cos45, sin45) ≈ (7.071, 7.071).
        let c = 10.0 * std::f64::consts::FRAC_1_SQRT_2;
        assert!((d[0].x - c).abs() < 1e-9 && (d[0].y - c).abs() < 1e-9, "fue {:?}", d[0]);
    }
}
