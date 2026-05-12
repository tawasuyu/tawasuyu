//! Geometría de la chacana andina (cruz cuadrada escalonada).
//!
//! Genera un polígono cerrado de 20 vértices: un cuadrado central,
//! cuatro escalones (uno por brazo cardinal) y cuatro puntas que
//! extienden hasta `arm_extent`.
//!
//! Convención: plano XY, centro en `(0, 0)`, +Y hacia el norte,
//! +X hacia el este. Toda la API es pura: ningún I/O, ninguna asignación
//! global; apta para ejecutar dentro de un shader-host, en un test,
//! o en una integración nativa.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChacanaSpec {
    /// Distancia desde el centro hasta la punta del brazo.
    pub arm_extent: f32,
    /// Semi-grosor del brazo. El escalón mide `2 * thickness`.
    pub thickness: f32,
}

impl ChacanaSpec {
    /// Configuración canónica: brazo 1.0, grosor 0.18 (proporciones del logo).
    pub const CLASSIC: Self = Self {
        arm_extent: 1.0,
        thickness: 0.18,
    };

    pub const fn new(arm_extent: f32, thickness: f32) -> Self {
        Self {
            arm_extent,
            thickness,
        }
    }

    /// Las cuatro puntas cardinales en orden `[N, E, S, W]`.
    /// Coordenadas listas para anclar UI sobre la chacana.
    pub fn tips(&self) -> [(f32, f32); 4] {
        let l = self.arm_extent;
        [(0.0, l), (l, 0.0), (0.0, -l), (-l, 0.0)]
    }

    /// Bounding box axis-aligned `(min, max)`.
    pub fn aabb(&self) -> ((f32, f32), (f32, f32)) {
        let l = self.arm_extent;
        ((-l, -l), (l, l))
    }

    /// Perímetro cerrado en orden horario: 20 vértices, listo para `LINE_LOOP`.
    pub fn perimeter(&self) -> Vec<(f32, f32)> {
        let s = self.thickness;
        let l = self.arm_extent;
        let s2 = s * 2.0;
        vec![
            (s, l),
            (s, s2),
            (s2, s2),
            (s2, s),
            (l, s),
            (l, -s),
            (s2, -s),
            (s2, -s2),
            (s, -s2),
            (s, -l),
            (-s, -l),
            (-s, -s2),
            (-s2, -s2),
            (-s2, -s),
            (-l, -s),
            (-l, s),
            (-s2, s),
            (-s2, s2),
            (-s, s2),
            (-s, l),
        ]
    }

    /// Triangulación: 9 rectángulos (1 centro + 4 escalones + 4 puntas) = 54 vértices.
    /// Listo para `GL_TRIANGLES`.
    pub fn triangles(&self) -> Vec<(f32, f32)> {
        let s = self.thickness;
        let l = self.arm_extent;
        let s2 = s * 2.0;
        let mut tri = Vec::with_capacity(9 * 6);
        let mut rect = |x0: f32, y0: f32, x1: f32, y1: f32| {
            tri.push((x0, y0));
            tri.push((x1, y0));
            tri.push((x1, y1));
            tri.push((x0, y0));
            tri.push((x1, y1));
            tri.push((x0, y1));
        };
        // Cuadrado central
        rect(-s, -s, s, s);
        // Escalones (un rect 4s × s por brazo)
        rect(-s2, s, s2, s2); // N
        rect(-s2, -s2, s2, -s); // S
        rect(s, -s2, s2, s2); // E
        rect(-s2, -s2, -s, s2); // W
        // Puntas (un rect 2s × (l - 2s) por brazo)
        rect(-s, s2, s, l); // N
        rect(-s, -l, s, -s2); // S
        rect(s2, -s, l, s); // E
        rect(-l, -s, -s2, s); // W
        tri
    }

    /// Para un punto cualquiera, devuelve la punta más cercana y su distancia.
    /// Útil para snapping de interacción.
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
    fn perimeter_has_20_vertices() {
        assert_eq!(ChacanaSpec::CLASSIC.perimeter().len(), 20);
    }

    #[test]
    fn triangles_form_9_rectangles() {
        assert_eq!(ChacanaSpec::CLASSIC.triangles().len(), 9 * 6);
    }

    #[test]
    fn tips_match_cardinals() {
        let c = ChacanaSpec::new(2.0, 0.3);
        let tips = c.tips();
        assert_eq!(tips[0], (0.0, 2.0)); // N
        assert_eq!(tips[1], (2.0, 0.0)); // E
        assert_eq!(tips[2], (0.0, -2.0)); // S
        assert_eq!(tips[3], (-2.0, 0.0)); // W
    }

    #[test]
    fn closest_tip_to_upper_left_is_north() {
        let c = ChacanaSpec::CLASSIC;
        let (tip, _d) = c.closest_tip((-0.1, 0.95));
        assert_eq!(tip, (0.0, 1.0));
    }

    #[test]
    fn aabb_matches_extent() {
        let c = ChacanaSpec::new(1.5, 0.2);
        assert_eq!(c.aabb(), ((-1.5, -1.5), (1.5, 1.5)));
    }
}
