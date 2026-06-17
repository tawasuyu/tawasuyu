//! Geometría de mallas: el vértice 3D ([`Vertex3d`]), un cubo de prueba
//! ([`cube`]) y un compositor de cajas transformadas ([`push_cube`]) para armar
//! mallas multi-caja en CPU — p.ej. un **muñeco articulado** (cabeza/torso/
//! miembros como cajas rotadas en sus articulaciones).
//!
//! Sigue el idiom de `llimphi-raster::gpu` (subir a GPU vía `to_ne_bytes`, sin
//! `bytemuck`) para no agregar una dependencia nueva al workspace.

use glam::{Mat4, Vec3};

/// Vértice 3D: posición en mundo + color RGB lineal.
#[derive(Debug, Clone, Copy)]
pub struct Vertex3d {
    pub pos: [f32; 3],
    pub color: [f32; 3],
}

impl Vertex3d {
    /// Tamaño en bytes de un vértice empaquetado (`6 × f32`).
    pub const SIZE: usize = 6 * 4;

    /// Vuelca este vértice al buffer en orden `pos.xyz, color.rgb` (native
    /// endian, como hace `GpuBatch`).
    pub fn write_to(&self, out: &mut Vec<u8>) {
        for v in self.pos {
            out.extend_from_slice(&v.to_ne_bytes());
        }
        for v in self.color {
            out.extend_from_slice(&v.to_ne_bytes());
        }
    }
}

/// Las 8 esquinas del cubo unitario centrado en el origen (lado 1, `-0.5..0.5`).
const CUBE_CORNERS: [[f32; 3]; 8] = [
    [-0.5, -0.5, -0.5],
    [0.5, -0.5, -0.5],
    [0.5, 0.5, -0.5],
    [-0.5, 0.5, -0.5],
    [-0.5, -0.5, 0.5],
    [0.5, -0.5, 0.5],
    [0.5, 0.5, 0.5],
    [-0.5, 0.5, 0.5],
];

/// Los 36 índices (12 triángulos) del cubo, winding CCW visto desde afuera.
#[rustfmt::skip]
pub const CUBE_INDICES: [u16; 36] = [
    0, 2, 1, 0, 3, 2, // -Z (atrás)
    4, 5, 6, 4, 6, 7, // +Z (frente)
    0, 4, 7, 0, 7, 3, // -X (izquierda)
    1, 2, 6, 1, 6, 5, // +X (derecha)
    0, 1, 5, 0, 5, 4, // -Y (abajo)
    3, 7, 6, 3, 6, 2, // +Y (arriba)
];

/// Cubo unitario centrado en el origen (lado 1, de `-0.5` a `0.5`). 8 vértices
/// coloreados por su posición (`color = pos + 0.5`) → un degradé que deja ver
/// las tres caras visibles distintas. 36 índices (12 triángulos), winding CCW.
pub fn cube() -> (Vec<Vertex3d>, Vec<u16>) {
    let verts = CUBE_CORNERS
        .iter()
        .map(|&[x, y, z]| Vertex3d {
            pos: [x, y, z],
            color: [x + 0.5, y + 0.5, z + 0.5],
        })
        .collect();
    (verts, CUBE_INDICES.to_vec())
}

/// Apila un cubo transformado por `m` (mapea el cubo unitario `[-0.5,0.5]³` a su
/// caja en mundo) con color plano `color`, en `verts`/`indices`. Es el ladrillo
/// para componer mallas multi-caja en CPU: cada llamada agrega 8 vértices + 36
/// índices con la base reubicada. Para un miembro articulado, `m` suele ser
/// `T(articulación) · R(ángulo) · T(0,-largo/2,0) · S(tamaño)`.
pub fn push_cube(verts: &mut Vec<Vertex3d>, indices: &mut Vec<u16>, m: Mat4, color: [f32; 3]) {
    let base = verts.len() as u16;
    for c in CUBE_CORNERS {
        let p = m.transform_point3(Vec3::from_array(c));
        verts.push(Vertex3d { pos: p.to_array(), color });
    }
    for i in CUBE_INDICES {
        indices.push(base + i);
    }
}
