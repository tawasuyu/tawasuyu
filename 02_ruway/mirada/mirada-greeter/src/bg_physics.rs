//! Fondo físico del greeter: tentáculos esqueletales vivos.
//!
//! A diferencia de los fondos procedurales (stateless), éste tiene **estado**:
//! una simulación Verlet (`llimphi_anim::physics`) que mueve cadenas de huesos
//! bajo gravedad + viento, con el skinning deformando mallas. Se **stepea** en
//! el loop del greeter (`RainTick`) y se pinta en el fondo. Cuelgan del borde
//! superior y se mecen perpetuamente con un viento senoidal — vivo aunque nadie
//! toque nada (un lock screen no tiene foco de cursor garantizado).
//!
//! Trabaja en un **espacio virtual** 16:9 fijo; al pintar se escala al rect real
//! de la ventana (estiramiento ~uniforme en pantallas 16:9).

use llimphi_anim::physics::{pose_chain_from_points, Physics};
use llimphi_anim::skel::{BoneId, Mesh, Pose, Skeleton, Vertex};
use llimphi_mesh::paint_solid;
use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, Vec2};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::vello::Scene;
use llimphi_ui::PaintRect;

const VW: f64 = 1600.0;
const VH: f64 = 900.0;
const SEGS: usize = 9;
const SEG_LEN: f64 = 46.0;
const THICK: f64 = 30.0;

struct Rope {
    phys: Physics,
    skel: Skeleton,
    mesh: Mesh,
    bones: Vec<BoneId>,
    color: Color,
}

fn make_rope(anchor: Point, color: Color) -> Rope {
    let mut phys = Physics::new();
    phys.floor_y = Some(VH - 30.0);
    let mut prev = phys.particle(anchor, true);
    // SEGS+1 huesos necesitan SEGS+2 puntos: `pose_chain_from_points` pide
    // `points.len() >= bones.len()+1` (el último hueso toma su ángulo del segmento
    // hacia el punto siguiente). Sin esa partícula-cola extra el guard cortaba y
    // el esqueleto nunca seguía a la física → el mesh quedaba en bind pose.
    for i in 1..=SEGS + 1 {
        let p = phys.particle(Point::new(anchor.x, anchor.y + i as f64 * SEG_LEN), false);
        phys.link(prev, p);
        prev = p;
    }

    let mut skel = Skeleton::new();
    let mut bones = vec![skel.add_bone(None, Pose::identity())];
    for _ in 1..=SEGS {
        bones.push(skel.add_bone(Some(*bones.last().unwrap()), Pose::translate(Vec2::new(SEG_LEN, 0.0))));
    }
    skel.bind();

    let mut mesh = Mesh::new();
    for i in 0..=SEGS {
        let x = i as f64 * SEG_LEN;
        let half = THICK * 0.5 * (1.0 - 0.7 * (i as f64 / SEGS as f64));
        mesh.vertices.push(Vertex::rigid(Point::new(x, -half), (0.0, 0.0), bones[i]));
        mesh.vertices.push(Vertex::rigid(Point::new(x, half), (0.0, 1.0), bones[i]));
    }
    for i in 0..SEGS {
        let (t0, t1) = ((2 * i) as u32, (2 * (i + 1)) as u32);
        let (b0, b1) = ((2 * i + 1) as u32, (2 * (i + 1) + 1) as u32);
        mesh.triangles.push([t0, t1, b1]);
        mesh.triangles.push([t0, b1, b0]);
    }

    Rope { phys, skel, mesh, bones, color }
}

/// Mezcla `c` hacia blanco/negro por `f` (negativo = más oscuro).
fn shade(base: (u8, u8, u8), f: f64, alpha: u8) -> Color {
    let mix = |c: u8| {
        let v = c as f64;
        let t = if f >= 0.0 { v + (255.0 - v) * f } else { v * (1.0 + f) };
        t.clamp(0.0, 255.0) as u8
    };
    Color::from_rgba8(mix(base.0), mix(base.1), mix(base.2), alpha)
}

/// Fondo de física: varios tentáculos colgando del techo.
pub struct PhysicsBg {
    ropes: Vec<Rope>,
    t: f64,
}

impl PhysicsBg {
    /// Construye el fondo con tentáculos repartidos a lo ancho, coloreados a
    /// partir del color base del tema (`bright`).
    pub fn new(bright: (u8, u8, u8)) -> Self {
        let count = 8;
        let ropes = (0..count)
            .map(|i| {
                let x = VW * (i as f64 + 0.5) / count as f64;
                // Profundidad: alterna claros/oscuros y semitransparentes.
                let f = -0.25 + 0.45 * ((i % 3) as f64 / 2.0);
                let alpha = 150 + (i % 2) as u8 * 70;
                make_rope(Point::new(x, -20.0), shade(bright, f, alpha))
            })
            .collect();
        Self { ropes, t: 0.0 }
    }

    /// Avanza la simulación `dt` segundos. Un viento senoidal mece los tentáculos
    /// de forma perpetua (no depende del cursor).
    pub fn step(&mut self, dt: f64) {
        self.t += dt;
        let wind = (self.t * 0.7).sin() * 240.0 + (self.t * 1.9 + 1.0).sin() * 90.0;
        for rope in &mut self.ropes {
            rope.phys.gravity = Vec2::new(wind, 900.0);
            rope.phys.step(dt, 6);
            let pts = rope.phys.positions();
            pose_chain_from_points(&mut rope.skel, &rope.bones, &pts);
        }
    }

    /// Datos de dibujo de este frame (malla + posiciones deformadas + color),
    /// para mover a un closure de pintura `'static`. Se llama en `view`
    /// (read-only) y se pinta con [`paint_snapshot`].
    pub fn snapshot(&self) -> Vec<RopeDraw> {
        self.ropes
            .iter()
            .map(|r| (r.mesh.clone(), r.mesh.deform(&r.skel), r.color))
            .collect()
    }
}

/// Una malla deformada lista para pintar (malla, posiciones, color).
pub type RopeDraw = (Mesh, Vec<Point>, Color);

/// Pinta un snapshot escalando el espacio virtual al `rect` real de la ventana.
pub fn paint_snapshot(snap: &[RopeDraw], scene: &mut Scene, rect: PaintRect) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let xform = Affine::translate((rect.x as f64, rect.y as f64))
        * Affine::scale_non_uniform(rect.w as f64 / VW, rect.h as f64 / VH);
    for (mesh, pos, color) in snap {
        paint_solid(scene, mesh, pos, xform, *color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produce_geometria_viva_y_finita() {
        let mut bg = PhysicsBg::new((180, 200, 255));
        // Snapshot inicial: 8 tentáculos, mallas con triángulos, posiciones finitas.
        let snap0 = bg.snapshot();
        assert_eq!(snap0.len(), 8, "deben ser 8 tentáculos");
        for (mesh, pos, _) in &snap0 {
            assert!(!mesh.triangles.is_empty(), "la malla debe tener triángulos");
            assert_eq!(pos.len(), mesh.vertices.len(), "una posición por vértice");
            assert!(
                pos.iter().all(|p| p.x.is_finite() && p.y.is_finite()),
                "posiciones finitas"
            );
        }
        // Tras stepear con viento (~4 s), la física mueve los tentáculos (vivo) y
        // NO explota a infinito/NaN.
        for _ in 0..120 {
            bg.step(1.0 / 30.0);
        }
        let snap1 = bg.snapshot();
        let (a, b) = (snap0[0].1[snap0[0].1.len() - 1], snap1[0].1[snap1[0].1.len() - 1]);
        assert!(
            (a.x - b.x).abs() + (a.y - b.y).abs() > 1.0,
            "la física debe mover los tentáculos (vivo)"
        );
        for (_, pos, _) in &snap1 {
            assert!(
                pos.iter().all(|p| p.x.is_finite() && p.y.is_finite() && p.x.abs() < 1e5 && p.y.abs() < 1e5),
                "la física no debe explotar a infinito/NaN"
            );
        }
        // No es la astilla degenerada en (0,0): los tentáculos se reparten a lo
        // ancho y cuelgan en alto → caja envolvente grande sobre el lienzo virtual.
        let all: Vec<Point> = snap1.iter().flat_map(|(_, p, _)| p.iter().copied()).collect();
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in &all {
            x0 = x0.min(p.x); y0 = y0.min(p.y); x1 = x1.max(p.x); y1 = y1.max(p.y);
        }
        assert!(x1 - x0 > VW * 0.5, "los tentáculos deben repartirse a lo ancho (no apilados en 0,0)");
        assert!(y1 - y0 > VH * 0.3, "los tentáculos deben colgar en alto");
    }
}
