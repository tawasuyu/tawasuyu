//! Física de partículas (Verlet) + constraints de distancia, para mover
//! esqueletos con **leyes físicas**: cuerdas, péndulos, ragdolls que caen y se
//! balancean bajo gravedad, con piso y paredes.
//!
//! El patrón es: una cadena de [`Particle`]s unidas por [`Link`]s (distancias
//! rígidas = longitudes de hueso) se integra con gravedad; luego
//! [`pose_chain_from_points`] convierte las posiciones de las partículas en poses
//! de una cadena de huesos, y el skinning deforma la malla. Así la física maneja
//! el esqueleto, no una animación keyframeada.
//!
//! Verlet (en vez de Euler con velocidades explícitas) hace los constraints de
//! distancia triviales y estables: se resuelven por proyección iterativa.

use kurbo::{Point, Vec2};

use crate::skel::{BoneId, Skeleton};

/// Una partícula puntual. `inv_mass = 0` la fija (pin); `damping` global la frena.
#[derive(Debug, Clone, Copy)]
pub struct Particle {
    pub pos: Point,
    pub prev: Point,
    pub inv_mass: f64,
}

impl Particle {
    pub fn new(pos: Point, pinned: bool) -> Self {
        Self {
            pos,
            prev: pos,
            inv_mass: if pinned { 0.0 } else { 1.0 },
        }
    }
    pub fn pinned(&self) -> bool {
        self.inv_mass == 0.0
    }
}

/// Restricción de distancia entre dos partículas (un "hueso" rígido).
#[derive(Debug, Clone, Copy)]
pub struct Link {
    pub a: usize,
    pub b: usize,
    pub rest: f64,
    /// 0..1: 1 = rígido, <1 = elástico.
    pub stiffness: f64,
}

/// Mundo físico 2D: partículas + links + gravedad + piso/paredes.
#[derive(Debug, Clone)]
pub struct Physics {
    pub particles: Vec<Particle>,
    pub links: Vec<Link>,
    pub gravity: Vec2,
    /// Factor de retención de velocidad por step (0..1, ~0.99).
    pub damping: f64,
    /// Si está, las partículas no bajan de esta `y` (piso).
    pub floor_y: Option<f64>,
    /// Paredes `(x0, x1)` que contienen en x.
    pub walls_x: Option<(f64, f64)>,
}

impl Default for Physics {
    fn default() -> Self {
        Self {
            particles: Vec::new(),
            links: Vec::new(),
            gravity: Vec2::new(0.0, 980.0), // px/s² hacia abajo (y crece para abajo)
            damping: 0.99,
            floor_y: None,
            walls_x: None,
        }
    }
}

impl Physics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Agrega una partícula y devuelve su índice.
    pub fn particle(&mut self, pos: Point, pinned: bool) -> usize {
        let i = self.particles.len();
        self.particles.push(Particle::new(pos, pinned));
        i
    }

    /// Une dos partículas con `rest` = su distancia actual y rigidez total.
    pub fn link(&mut self, a: usize, b: usize) {
        let rest = (self.particles[b].pos - self.particles[a].pos).hypot();
        self.links.push(Link { a, b, rest, stiffness: 1.0 });
    }

    /// Une dos partículas con una distancia de reposo y rigidez dadas.
    pub fn link_with(&mut self, a: usize, b: usize, rest: f64, stiffness: f64) {
        self.links.push(Link { a, b, rest, stiffness });
    }

    /// Empuja las partículas lejos de `center` dentro de `radius` (campo de
    /// repulsión — p. ej. el cursor barriendo las cuerdas).
    pub fn repel(&mut self, center: Point, radius: f64, strength: f64) {
        for p in &mut self.particles {
            if p.pinned() {
                continue;
            }
            let d = p.pos - center;
            let dist = d.hypot();
            if dist < radius && dist > 1e-6 {
                let push = (1.0 - dist / radius) * strength;
                p.pos += d * (push / dist);
            }
        }
    }

    /// Un paso de simulación: integra (gravedad + inercia) y resuelve los
    /// constraints `iterations` veces (más iteraciones = más rígido/estable).
    pub fn step(&mut self, dt: f64, iterations: usize) {
        let dt2 = dt * dt;
        for p in &mut self.particles {
            if p.pinned() {
                continue;
            }
            let vel = (p.pos - p.prev) * self.damping;
            p.prev = p.pos;
            p.pos = p.pos + vel + self.gravity * dt2;
        }
        for _ in 0..iterations.max(1) {
            self.solve_links();
            self.solve_bounds();
        }
    }

    fn solve_links(&mut self) {
        for k in 0..self.links.len() {
            let Link { a, b, rest, stiffness } = self.links[k];
            let pa = self.particles[a].pos;
            let pb = self.particles[b].pos;
            let wa = self.particles[a].inv_mass;
            let wb = self.particles[b].inv_mass;
            let wsum = wa + wb;
            if wsum < 1e-12 {
                continue;
            }
            let delta = pb - pa;
            let d = delta.hypot();
            if d < 1e-9 {
                continue;
            }
            let diff = (d - rest) / d * stiffness;
            let corr = delta * diff;
            self.particles[a].pos = pa + corr * (wa / wsum);
            self.particles[b].pos = pb - corr * (wb / wsum);
        }
    }

    fn solve_bounds(&mut self) {
        for p in &mut self.particles {
            if p.pinned() {
                continue;
            }
            if let Some(fy) = self.floor_y {
                if p.pos.y > fy {
                    p.pos.y = fy;
                    // Fricción: amortigua el deslizamiento horizontal en el piso.
                    p.prev.x = p.pos.x + (p.prev.x - p.pos.x) * 0.5;
                }
            }
            if let Some((x0, x1)) = self.walls_x {
                p.pos.x = p.pos.x.clamp(x0, x1);
            }
        }
    }

    /// Posiciones actuales de las partículas (para construir poses / pintar).
    pub fn positions(&self) -> Vec<Point> {
        self.particles.iter().map(|p| p.pos).collect()
    }
}

/// Posa una cadena de huesos para que siga una cadena de puntos (las partículas
/// físicas). `points` tiene `bones.len() + 1` entradas: el hueso `i` va de
/// `points[i]` a `points[i+1]`. Setea la translación de la raíz a `points[0]` y
/// las rotaciones a los ángulos de cada segmento (relativos en la jerarquía).
/// Llama `update`. Es el puente física → esqueleto.
pub fn pose_chain_from_points(skel: &mut Skeleton, bones: &[BoneId], points: &[Point]) {
    if bones.is_empty() || points.len() < bones.len() + 1 {
        return;
    }
    let seg_angle = |i: usize| {
        let d = points[i + 1] - points[i];
        if d.hypot() < 1e-9 {
            0.0
        } else {
            d.y.atan2(d.x)
        }
    };

    // Raíz: su origen va a points[0]; rotación = ángulo del primer segmento.
    let root = bones[0];
    let mut rp = skel.pose(root);
    rp.translation = points[0].to_vec2();
    rp.rotation = seg_angle(0);
    skel.set_pose(root, rp);

    // Eslabones: rotación local = ángulo del segmento − ángulo del anterior
    // (la rotación world se acumula por la jerarquía).
    for i in 1..bones.len() {
        let mut p = skel.pose(bones[i]);
        p.rotation = seg_angle(i) - seg_angle(i - 1);
        skel.set_pose(bones[i], p);
    }
    skel.update();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skel::{Pose, Skeleton};

    #[test]
    fn la_gravedad_hace_caer_una_particula_libre() {
        let mut w = Physics::new();
        let i = w.particle(Point::new(0.0, 0.0), false);
        w.step(0.1, 1);
        assert!(w.particles[i].pos.y > 0.0, "debería caer (y crece)");
    }

    #[test]
    fn una_particula_fija_no_se_mueve() {
        let mut w = Physics::new();
        let i = w.particle(Point::new(5.0, 5.0), true);
        for _ in 0..10 {
            w.step(0.1, 2);
        }
        assert_eq!(w.particles[i].pos, Point::new(5.0, 5.0));
    }

    #[test]
    fn un_pendulo_cuelga_a_la_distancia_de_reposo() {
        let mut w = Physics::new();
        let anchor = w.particle(Point::new(0.0, 0.0), true);
        let bob = w.particle(Point::new(0.0, 50.0), false);
        w.link(anchor, bob); // rest = 50
        for _ in 0..400 {
            w.step(1.0 / 120.0, 8);
        }
        // En reposo cuelga recto hacia abajo a ~50 del ancla.
        let p = w.particles[bob].pos;
        assert!((p.x).abs() < 1.0, "x ~ 0, fue {}", p.x);
        assert!((p.y - 50.0).abs() < 1.0, "y ~ 50, fue {}", p.y);
    }

    #[test]
    fn el_piso_detiene_la_caida() {
        let mut w = Physics::new();
        w.floor_y = Some(100.0);
        let i = w.particle(Point::new(0.0, 0.0), false);
        for _ in 0..200 {
            w.step(1.0 / 120.0, 2);
        }
        assert!(w.particles[i].pos.y <= 100.0 + 1e-6, "no pasa el piso");
        assert!(w.particles[i].pos.y > 90.0, "llegó cerca del piso");
    }

    #[test]
    fn pose_chain_sigue_los_puntos() {
        // Cadena de 2 huesos; puntos en L vertical-luego-horizontal.
        let mut s = Skeleton::new();
        let a = s.add_bone(None, Pose::translate(Vec2::new(0.0, 0.0)));
        let b = s.add_bone(Some(a), Pose::translate(Vec2::new(50.0, 0.0)));
        s.bind();
        let points = [
            Point::new(0.0, 0.0),
            Point::new(0.0, 50.0),  // primer segmento apunta hacia abajo (+y)
            Point::new(50.0, 50.0), // segundo apunta a la derecha (+x)
        ];
        pose_chain_from_points(&mut s, &[a, b], &points);
        // El origen del hueso b debe estar en points[1]; su punta (local 50,0) en points[2].
        let b_origin = s.world(b) * Point::ZERO;
        assert!((b_origin.x - 0.0).abs() < 1e-6 && (b_origin.y - 50.0).abs() < 1e-6, "{b_origin:?}");
        let tip = s.world(b) * Point::new(50.0, 0.0);
        assert!((tip.x - 50.0).abs() < 1e-6 && (tip.y - 50.0).abs() < 1e-6, "tip {tip:?}");
    }
}
