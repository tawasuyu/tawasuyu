//! Geometría de la chacana andina escalonada (cruz cuadrada de Tiwanaku).
//!
//! Modelo paramétrico: un cuadrado central de lado `2 * center_half()`,
//! del que sobresalen cuatro brazos cardinales formados por `steps`
//! niveles. Cada nivel adelgaza al brazo en `thickness` por lado y lo
//! prolonga en `thickness` hacia afuera.
//!
//! Para `steps = 2` (clásica mística):
//! - Centro: cuadrado `6s × 6s` (donde `s = thickness`).
//! - Nivel 1: rectángulo perpendicular `4s × s` adosado a cada cara del centro.
//! - Nivel 2 (punta): rectángulo `2s × s` adosado al nivel 1.
//!
//! Resultado: bounding box `±5s` (cuadrado, no alargado como una cruz latina),
//! 9 rectángulos disjuntos triangulables, 4 tips cardinales.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChacanaSpec {
    /// Unidad base de la geometría. Cada paso aporta `thickness` de ancho
    /// y `thickness` de profundidad.
    pub thickness: f32,
    /// Cantidad de escalones por brazo (`>= 1`). La chacana mística clásica = `2`.
    pub steps: u32,
}

impl ChacanaSpec {
    /// Configuración canónica del logo Tawasuyu: 2 escalones, thickness 0.13
    /// (bounding box ≈ 1.30 × 1.30 en unidades de mundo).
    pub const CLASSIC: Self = Self {
        thickness: 0.13,
        steps: 2,
    };

    pub const fn new(thickness: f32, steps: u32) -> Self {
        Self { thickness, steps }
    }

    /// Semi-lado del cuadrado central — la parte **más ancha** de la chacana.
    pub fn center_half(&self) -> f32 {
        (self.steps as f32 + 1.0) * self.thickness
    }

    /// Distancia desde el centro a la punta más externa.
    pub fn arm_extent(&self) -> f32 {
        self.center_half() + self.steps as f32 * self.thickness
    }

    /// Las cuatro puntas cardinales `[N, E, S, W]`.
    pub fn tips(&self) -> [(f32, f32); 4] {
        let l = self.arm_extent();
        [(0.0, l), (l, 0.0), (0.0, -l), (-l, 0.0)]
    }

    pub fn aabb(&self) -> ((f32, f32), (f32, f32)) {
        let l = self.arm_extent();
        ((-l, -l), (l, l))
    }

    /// Triangulación: `1 + 4 * steps` rectángulos en `GL_TRIANGLES`.
    /// Para `steps = 2`: 9 rects = 54 vértices.
    pub fn triangles(&self) -> Vec<(f32, f32)> {
        let s = self.thickness;
        let c = self.center_half();
        let mut tri = Vec::with_capacity(6 * (1 + 4 * self.steps as usize));
        let mut rect = |x0: f32, y0: f32, x1: f32, y1: f32| {
            tri.push((x0, y0));
            tri.push((x1, y0));
            tri.push((x1, y1));
            tri.push((x0, y0));
            tri.push((x1, y1));
            tri.push((x0, y1));
        };
        rect(-c, -c, c, c);
        for k in 1..=self.steps {
            // El k-ésimo nivel (1 = más cerca del centro, steps = punta)
            // adelgaza a (steps - k + 1) * thickness de semi-ancho.
            let hw = (self.steps - k + 1) as f32 * s;
            let inner = c + (k - 1) as f32 * s;
            let outer = c + k as f32 * s;
            rect(-hw, inner, hw, outer); // N
            rect(-hw, -outer, hw, -inner); // S
            rect(inner, -hw, outer, hw); // E
            rect(-outer, -hw, -inner, hw); // W
        }
        tri
    }

    /// Para un punto cualquiera, devuelve la punta más cercana y la distancia.
    pub fn closest_tip(&self, p: (f32, f32)) -> ((f32, f32), f32) {
        let tips = self.tips();
        let mut best = (tips[0], f32::INFINITY);
        for t in tips.iter() {
            let dx = t.0 - p.0;
            let dy = t.1 - p.1;
            let d = (dx * dx + dy * dy).sqrt();
            if d < best.1 {
                best = (*t, d);
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_is_two_step_chacana() {
        let c = ChacanaSpec::CLASSIC;
        assert_eq!(c.steps, 2);
        // center_half = 3 * 0.13 = 0.39; arm_extent = 0.65.
        assert!((c.center_half() - 0.39).abs() < 1e-6);
        assert!((c.arm_extent() - 0.65).abs() < 1e-6);
    }

    #[test]
    fn arm_extent_grows_with_steps() {
        let c1 = ChacanaSpec::new(0.1, 1);
        let c2 = ChacanaSpec::new(0.1, 2);
        let c3 = ChacanaSpec::new(0.1, 3);
        assert!(c1.arm_extent() < c2.arm_extent());
        assert!(c2.arm_extent() < c3.arm_extent());
    }

    #[test]
    fn triangles_one_rect_plus_four_per_step() {
        let c1 = ChacanaSpec::new(0.1, 1);
        assert_eq!(c1.triangles().len(), 6 * (1 + 4 * 1));
        let c2 = ChacanaSpec::CLASSIC;
        assert_eq!(c2.triangles().len(), 6 * (1 + 4 * 2));
        let c3 = ChacanaSpec::new(0.1, 3);
        assert_eq!(c3.triangles().len(), 6 * (1 + 4 * 3));
    }

    #[test]
    fn tips_match_cardinals() {
        let c = ChacanaSpec::CLASSIC;
        let l = c.arm_extent();
        let tips = c.tips();
        assert_eq!(tips[0], (0.0, l)); // N
        assert_eq!(tips[1], (l, 0.0)); // E
        assert_eq!(tips[2], (0.0, -l)); // S
        assert_eq!(tips[3], (-l, 0.0)); // W
    }

    #[test]
    fn closest_tip_to_upper_point_is_north() {
        let c = ChacanaSpec::CLASSIC;
        let (tip, _d) = c.closest_tip((-0.1, 0.55));
        assert_eq!(tip, (0.0, c.arm_extent()));
    }

    #[test]
    fn aabb_matches_extent() {
        let c = ChacanaSpec::new(0.12, 2);
        let l = c.arm_extent();
        assert_eq!(c.aabb(), ((-l, -l), (l, l)));
    }
}
