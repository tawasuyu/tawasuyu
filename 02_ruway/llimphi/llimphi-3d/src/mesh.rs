//! Geometría de prueba para M0: un cubo indexado con color por vértice.
//!
//! Sigue el idiom de `llimphi-raster::gpu` (subir a GPU vía `to_ne_bytes`, sin
//! `bytemuck`) para no agregar una dependencia nueva al workspace.

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

/// Cubo unitario centrado en el origen (lado 1, de `-0.5` a `0.5`). 8 vértices
/// coloreados por su posición (`color = pos + 0.5`) → un degradé que deja ver
/// las tres caras visibles distintas. 36 índices (12 triángulos), winding CCW.
pub fn cube() -> (Vec<Vertex3d>, Vec<u16>) {
    let mut verts = Vec::with_capacity(8);
    for &(x, y, z) in &[
        (-0.5, -0.5, -0.5),
        (0.5, -0.5, -0.5),
        (0.5, 0.5, -0.5),
        (-0.5, 0.5, -0.5),
        (-0.5, -0.5, 0.5),
        (0.5, -0.5, 0.5),
        (0.5, 0.5, 0.5),
        (-0.5, 0.5, 0.5),
    ] {
        verts.push(Vertex3d {
            pos: [x, y, z],
            color: [x + 0.5, y + 0.5, z + 0.5],
        });
    }
    // Caras CCW vistas desde afuera.
    #[rustfmt::skip]
    let indices: Vec<u16> = vec![
        // -Z (atrás)
        0, 2, 1, 0, 3, 2,
        // +Z (frente)
        4, 5, 6, 4, 6, 7,
        // -X (izquierda)
        0, 4, 7, 0, 7, 3,
        // +X (derecha)
        1, 2, 6, 1, 6, 5,
        // -Y (abajo)
        0, 1, 5, 0, 5, 4,
        // +Y (arriba)
        3, 7, 6, 3, 6, 2,
    ];
    (verts, indices)
}
