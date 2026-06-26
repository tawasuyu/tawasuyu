//! El **rig esqueletal**: la representación editable de una cadena de huesos con
//! una malla-tira auto-skinneada, que se **compila** a un
//! [`llimphi_anim::skel::Skeleton`] + [`llimphi_anim::skel::Mesh`] deformables.
//! Es la Fase 2 del studio: sobre el mismo runtime estilo Rive, pero ahora el
//! "clip" es una **deformación esqueletal** en vez de una máquina de estados.
//!
//! ## Qué autora (y qué no, todavía)
//!
//! Autora una **cadena** de huesos (cada uno con su largo y su ángulo de pose),
//! una malla-tubo generada paramétricamente alrededor de la cadena (skinning
//! suave en las articulaciones, igual que el `build_arm` canónico del demo
//! `lottie_rive_demo`), y un **IK de 2 huesos** opcional sobre los dos primeros
//! huesos persiguiendo un objetivo. El *weight-paint* a mano y la malla
//! arbitraria (importada/dibujada) quedan para F2.5/F3 — acá la malla es
//! derivada de la cadena, no editable vértice-a-vértice.
//!
//! La matemática (jerarquía, LBS, IK analítico) vive entera en `llimphi-anim`;
//! este módulo sólo **describe** el rig y lo proyecta al runtime, igual que
//! [`crate::doc::Doc`] hace con el `StateMachine`.

use llimphi_anim::constraint::solve_two_bone_ik;
use llimphi_anim::skel::{Mesh, Pose, Skeleton, Vertex, Weight};
use llimphi_ui::llimphi_raster::kurbo::{Point, Vec2};
use serde::{Deserialize, Serialize};

/// Un hueso de la cadena. El hueso apunta a lo largo de su eje local **+x**
/// (convención de `skel`/`constraint`): su hijo se traslada `len` en +x.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoneDef {
    /// Largo del hueso (distancia al hijo, en unidades de modelo).
    pub len: f64,
    /// Ángulo de pose **local** en radianes (lo que editan los sliders). En el
    /// bind pose la cadena está recta (todos los ángulos en 0).
    pub angle: f64,
}

impl BoneDef {
    pub fn new(len: f64) -> Self {
        BoneDef { len, angle: 0.0 }
    }
}

/// Cómo se genera la malla deformable alrededor de la cadena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeshMode {
    /// Tira-tubo a lo largo de la cadena (skinning rígido+blend en joints).
    /// Ideal para un miembro (brazo, cola).
    Tube,
    /// Rejilla rectangular que cubre toda la silueta, con cada vértice
    /// auto-skinneado a los huesos por distancia. Es la malla para **deformar
    /// una imagen/arte arbitrario** (sus UV mapean la textura completa).
    Grid,
}

/// El documento del rig: la cadena + parámetros de malla + IK + textura.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RigDoc {
    pub bones: Vec<BoneDef>,
    /// Medio-grosor de la malla-tubo (mitad de la altura de la tira).
    pub thickness: f64,
    /// Columnas de la malla por unidad de largo (densidad del tubo).
    pub cols: usize,
    /// Modo de generación de la malla.
    pub mesh_mode: MeshMode,
    /// Resolución de la rejilla (celdas a lo largo de la cadena, modo Grid).
    pub grid_res: usize,
    /// Relación alto/ancho de la rejilla (modo Grid); se ajusta al cargar una
    /// imagen para respetar su aspecto.
    pub mesh_aspect: f64,
    /// Path de la textura a deformar (se recarga al abrir el proyecto). Los
    /// píxeles NO se serializan — sólo la referencia al archivo.
    pub texture_path: Option<String>,
    /// ¿IK de 2 huesos activo sobre los huesos 0 y 1?
    pub ik_enabled: bool,
    /// Objetivo del IK (espacio de modelo).
    pub ik_target: (f64, f64),
    /// Solución de codo (flip de la rama del IK).
    pub ik_flip: bool,
}

impl Default for RigDoc {
    fn default() -> Self {
        RigDoc {
            bones: Vec::new(),
            thickness: 22.0,
            cols: 16,
            mesh_mode: MeshMode::Tube,
            grid_res: 10,
            mesh_aspect: 0.6,
            texture_path: None,
            ik_enabled: false,
            ik_target: (200.0, 40.0),
            ik_flip: false,
        }
    }
}

impl RigDoc {
    /// Rig de arranque: un brazo de 2 huesos, el ejemplo canónico.
    pub fn starter() -> Self {
        RigDoc {
            bones: vec![BoneDef::new(140.0), BoneDef::new(120.0)],
            ik_target: (180.0, 90.0),
            ..Default::default()
        }
    }

    /// Largo total de la cadena (suma de los huesos).
    pub fn total_len(&self) -> f64 {
        self.bones.iter().map(|b| b.len).sum()
    }

    /// Posición de arranque (arc-length) de cada hueso a lo largo de la cadena
    /// recta: `starts[i] = Σ len[0..i]`.
    fn starts(&self) -> Vec<f64> {
        let mut s = Vec::with_capacity(self.bones.len());
        let mut acc = 0.0;
        for b in &self.bones {
            s.push(acc);
            acc += b.len;
        }
        s
    }

    /// Construye el esqueleto en **bind pose** (cadena recta) y lo congela. El
    /// orden de los `BoneId` coincide con `self.bones`.
    fn build_skeleton_bind(&self) -> Skeleton {
        let mut s = Skeleton::new();
        let mut prev = None;
        for (i, _b) in self.bones.iter().enumerate() {
            // El hijo se traslada el largo del PADRE en +x; el root en el origen.
            let local = if i == 0 {
                Pose::identity()
            } else {
                Pose::translate(Vec2::new(self.bones[i - 1].len, 0.0))
            };
            prev = Some(s.add_bone(prev, local));
        }
        s.bind();
        s
    }

    /// Aplica los ángulos de pose actuales (y el IK, si está activo) sobre un
    /// esqueleto ya en bind, y lo deja `update()`-eado listo para deformar.
    fn pose_skeleton(&self, s: &mut Skeleton) {
        for (i, b) in self.bones.iter().enumerate() {
            let t = if i == 0 {
                Vec2::ZERO
            } else {
                Vec2::new(self.bones[i - 1].len, 0.0)
            };
            s.set_pose(i, Pose::new(t, b.angle, Vec2::new(1.0, 1.0)));
        }
        // IK sobre los dos primeros huesos: sobrescribe sus poses para que la
        // punta del hueso 1 alcance el objetivo. Los huesos ≥2 conservan su
        // ángulo de slider (relativo al hueso 1).
        if self.ik_enabled && self.bones.len() >= 2 {
            let tip_local = Vec2::new(self.bones[1].len, 0.0);
            let target = Point::new(self.ik_target.0, self.ik_target.1);
            solve_two_bone_ik(s, 0, 1, tip_local, target, self.ik_flip);
        }
        s.update();
    }

    /// Esqueleto posado (bind + poses + IK + update), listo para `deform`.
    pub fn skeleton(&self) -> Skeleton {
        let mut s = self.build_skeleton_bind();
        self.pose_skeleton(&mut s);
        s
    }

    /// La malla deformable según el modo activo.
    pub fn mesh(&self) -> Mesh {
        match self.mesh_mode {
            MeshMode::Tube => self.tube_mesh(),
            MeshMode::Grid => self.grid_mesh(),
        }
    }

    /// Malla-tubo skinneada alrededor de la cadena recta (bind space). Skinning
    /// suave en las articulaciones: lejos de un joint el vértice es rígido a su
    /// hueso; dentro de la ventana de blend mezcla con el hueso vecino.
    fn tube_mesh(&self) -> Mesh {
        let mut m = Mesh::new();
        let n = self.bones.len();
        if n == 0 {
            return m;
        }
        let total = self.total_len();
        let starts = self.starts();
        let cols = self.cols.max(2);
        let half = self.thickness;
        // Ventana de blend: una fracción del hueso más corto.
        let min_len = self.bones.iter().map(|b| b.len).fold(f64::MAX, f64::min);
        let blend = (min_len * 0.4).max(1.0);

        for i in 0..=cols {
            let p = total * i as f64 / cols as f64;
            let weights = self.weights_at(p, &starts, blend);
            let u = i as f64 / cols as f64;
            m.vertices.push(Vertex {
                rest: Point::new(p, -half),
                uv: (u, 0.0),
                weights: weights.clone(),
            });
            m.vertices.push(Vertex {
                rest: Point::new(p, half),
                uv: (u, 1.0),
                weights,
            });
        }
        for i in 0..cols {
            let (t0, t1) = ((2 * i) as u32, (2 * (i + 1)) as u32);
            let (b0, b1) = ((2 * i + 1) as u32, (2 * (i + 1) + 1) as u32);
            m.triangles.push([t0, t1, b1]);
            m.triangles.push([t0, b1, b0]);
        }
        m
    }

    /// Pesos de un vértice a arc-position `p`: rígido a su segmento, con blend
    /// lineal hacia el hueso vecino cerca de cada joint.
    fn weights_at(&self, p: f64, starts: &[f64], blend: f64) -> Vec<Weight> {
        let n = self.bones.len();
        // Segmento que contiene a p.
        let mut k = 0;
        while k + 1 < n && p >= starts[k + 1] {
            k += 1;
        }
        // Joint de entrada (con k-1) y de salida (con k+1).
        if k + 1 < n {
            let d = starts[k + 1] - p; // distancia al joint siguiente
            if d < blend {
                let t = (d / blend).clamp(0.0, 1.0);
                let wk = 0.5 + 0.5 * t;
                return vec![
                    Weight { bone: k, weight: wk },
                    Weight { bone: k + 1, weight: 1.0 - wk },
                ];
            }
        }
        if k > 0 {
            let d = p - starts[k]; // distancia al joint anterior
            if d < blend {
                let t = (d / blend).clamp(0.0, 1.0);
                let wk = 0.5 + 0.5 * t;
                return vec![
                    Weight { bone: k, weight: wk },
                    Weight { bone: k - 1, weight: 1.0 - wk },
                ];
            }
        }
        vec![Weight { bone: k, weight: 1.0 }]
    }

    /// Malla-rejilla que cubre la silueta (`[0,total] × [-H/2,H/2]`, con
    /// `H = total·aspect`), cada vértice **auto-skinneado** a los huesos por
    /// distancia (inverse-distance, top-2). Sus UV mapean la textura completa
    /// `0..1`, así que deforma una imagen arbitraria, no sólo un miembro.
    fn grid_mesh(&self) -> Mesh {
        let mut m = Mesh::new();
        let n = self.bones.len();
        if n == 0 {
            return m;
        }
        let total = self.total_len();
        let h = (total * self.mesh_aspect).max(1.0);
        let y_top = -h * 0.5;
        let starts = self.starts();
        let gx = self.grid_res.max(2);
        let gy = ((gx as f64 * self.mesh_aspect).round() as usize).max(2);

        for j in 0..=gy {
            for i in 0..=gx {
                let x = total * i as f64 / gx as f64;
                let y = y_top + h * j as f64 / gy as f64;
                let weights = self.skin_weights_at(Point::new(x, y), &starts);
                let uv = (i as f64 / gx as f64, j as f64 / gy as f64);
                m.vertices.push(Vertex {
                    rest: Point::new(x, y),
                    uv,
                    weights,
                });
            }
        }
        let stride = (gx + 1) as u32;
        for j in 0..gy as u32 {
            for i in 0..gx as u32 {
                let a = j * stride + i;
                let b = a + 1;
                let c = a + stride;
                let d = c + 1;
                m.triangles.push([a, b, d]);
                m.triangles.push([a, d, c]);
            }
        }
        m
    }

    /// Pesos auto-skin de un punto: distancia a cada segmento-hueso (en bind,
    /// recta sobre el eje x), inverse-distance², se queda con los 2 huesos más
    /// cercanos y normaliza. Da una deformación suave de la rejilla.
    fn skin_weights_at(&self, p: Point, starts: &[f64]) -> Vec<Weight> {
        let n = self.bones.len();
        // (bone, dist) por hueso.
        let mut ds: Vec<(usize, f64)> = (0..n)
            .map(|k| {
                let x0 = starts[k];
                let x1 = starts[k] + self.bones[k].len;
                let dx = if p.x < x0 {
                    x0 - p.x
                } else if p.x > x1 {
                    p.x - x1
                } else {
                    0.0
                };
                (k, dx.hypot(p.y))
            })
            .collect();
        // Top-2 más cercanos.
        ds.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        ds.truncate(2);
        let eps = 1e-3;
        let mut raw: Vec<(usize, f64)> =
            ds.iter().map(|(k, d)| (*k, 1.0 / (d * d + eps))).collect();
        let sum: f64 = raw.iter().map(|(_, w)| w).sum();
        if sum <= 0.0 {
            return vec![Weight { bone: 0, weight: 1.0 }];
        }
        for (_, w) in &mut raw {
            *w /= sum;
        }
        raw.into_iter()
            .map(|(bone, weight)| Weight { bone, weight })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compila_esqueleto_y_malla() {
        let rig = RigDoc::starter();
        let s = rig.skeleton();
        assert_eq!(s.len(), 2);
        let m = rig.mesh();
        assert!(!m.vertices.is_empty());
        assert!(!m.triangles.is_empty());
        // En bind (todos los ángulos 0, IK off) la deformación = reposo.
        let pos = m.deform(&s);
        assert_eq!(pos.len(), m.vertices.len());
        for (p, v) in pos.iter().zip(&m.vertices) {
            assert!((p.x - v.rest.x).abs() < 1e-6 && (p.y - v.rest.y).abs() < 1e-6);
        }
    }

    #[test]
    fn posar_un_hueso_deforma_la_malla() {
        let mut rig = RigDoc::starter();
        let rest = rig.mesh().deform(&rig.skeleton());
        // Doblar el codo (hueso 1) 0.8 rad.
        rig.bones[1].angle = 0.8;
        let bent = rig.mesh().deform(&rig.skeleton());
        // Al menos un vértice de la punta se movió respecto al reposo.
        let moved = rest
            .iter()
            .zip(&bent)
            .any(|(a, b)| (a.x - b.x).hypot(a.y - b.y) > 1.0);
        assert!(moved, "doblar el codo debería mover la malla");
    }

    #[test]
    fn grid_mesh_pesos_normalizados_y_deforma() {
        let mut rig = RigDoc::starter();
        rig.mesh_mode = MeshMode::Grid;
        rig.grid_res = 8;
        let m = rig.mesh();
        assert!(!m.vertices.is_empty() && !m.triangles.is_empty());
        // Cada vértice tiene pesos que suman ~1 (skinning bien normalizado).
        for v in &m.vertices {
            let s: f64 = v.weights.iter().map(|w| w.weight).sum();
            assert!((s - 1.0).abs() < 1e-6, "pesos deben sumar 1, fue {s}");
            assert!(v.weights.iter().all(|w| w.bone < rig.bones.len()));
        }
        // Posar el codo deforma la rejilla.
        let rest = m.deform(&rig.skeleton());
        rig.bones[1].angle = 0.9;
        let bent = rig.mesh().deform(&rig.skeleton());
        let moved = rest
            .iter()
            .zip(&bent)
            .any(|(a, b)| (a.x - b.x).hypot(a.y - b.y) > 1.0);
        assert!(moved, "doblar el codo debería deformar la rejilla");
    }

    #[test]
    fn ik_alcanza_el_objetivo() {
        let mut rig = RigDoc::starter();
        rig.ik_enabled = true;
        // Objetivo dentro del alcance (l1+l2 = 260).
        rig.ik_target = (150.0, 80.0);
        let s = rig.skeleton();
        // Punta del hueso 1 en mundo = world(1) * (len1, 0).
        let tip = s.world(1) * Point::new(rig.bones[1].len, 0.0);
        let target = Point::new(rig.ik_target.0, rig.ik_target.1);
        let err = (tip.x - target.x).hypot(tip.y - target.y);
        assert!(err < 1.0, "el IK debería alcanzar el objetivo, err={err}");
    }

    #[test]
    fn ik_clampa_objetivo_inalcanzable_sin_panickear() {
        let mut rig = RigDoc::starter();
        rig.ik_enabled = true;
        rig.ik_target = (10_000.0, 0.0); // mucho más lejos que el alcance
        let s = rig.skeleton();
        // No panickea y el brazo queda estirado hacia el objetivo (x≈260).
        let tip = s.world(1) * Point::new(rig.bones[1].len, 0.0);
        assert!(tip.x > 200.0, "brazo estirado hacia el objetivo lejano");
    }
}
