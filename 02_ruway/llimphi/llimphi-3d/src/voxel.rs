//! `VoxelGrid` — grid de voxels denso y acotado (CPU side). Cada voxel es
//! RGBA8: `rgb` = color, `a` = ocupación (`0` vacío, `>0` sólido). Se sube a
//! una textura 3D de GPU que el shader ray-march recorre por DDA.
//!
//! M1 es **denso** a propósito (lo más simple que funciona). El salto a sparse
//! (SVO/brickmap, saltar el aire) es M2 — ver `MOTOR-VOXEL.md` §11.2.

/// Grid denso de voxels RGBA8. Índice lineal `x + y*dx + z*dx*dy` (x contiguo),
/// que es justo el layout que espera `queue.write_texture` (filas en x, luego y,
/// luego capas en z).
#[derive(Clone)]
pub struct VoxelGrid {
    dim: [u32; 3],
    data: Vec<[u8; 4]>,
}

impl VoxelGrid {
    /// Grid vacío de `dim = [dx, dy, dz]` voxels.
    pub fn new(dim: [u32; 3]) -> Self {
        let n = (dim[0] * dim[1] * dim[2]) as usize;
        Self {
            dim,
            data: vec![[0, 0, 0, 0]; n],
        }
    }

    /// Dimensiones `[dx, dy, dz]`.
    pub fn dim(&self) -> [u32; 3] {
        self.dim
    }

    #[inline]
    fn idx(&self, x: u32, y: u32, z: u32) -> usize {
        (x + y * self.dim[0] + z * self.dim[0] * self.dim[1]) as usize
    }

    /// Marca un voxel sólido con color `rgb` (alpha = 255). Fuera de rango: no-op.
    pub fn set(&mut self, x: u32, y: u32, z: u32, rgb: [u8; 3]) {
        if x < self.dim[0] && y < self.dim[1] && z < self.dim[2] {
            let i = self.idx(x, y, z);
            self.data[i] = [rgb[0], rgb[1], rgb[2], 255];
        }
    }

    /// Vacía un voxel.
    pub fn clear(&mut self, x: u32, y: u32, z: u32) {
        if x < self.dim[0] && y < self.dim[1] && z < self.dim[2] {
            let i = self.idx(x, y, z);
            self.data[i] = [0, 0, 0, 0];
        }
    }

    /// Bytes RGBA planos listos para `queue.write_texture`.
    pub fn bytes(&self) -> &[u8] {
        // `[u8;4]` es contiguo: reinterpretamos el Vec como bytes planos.
        // SAFETY: `[u8;4]` no tiene padding; len*4 bytes válidos.
        unsafe {
            std::slice::from_raw_parts(self.data.as_ptr() as *const u8, self.data.len() * 4)
        }
    }

    /// Escena de prueba para M1: un piso de 2 capas + una esfera coloreada por
    /// posición flotando en el centro. Pone a prueba el DDA (atraviesa aire,
    /// pega en piso y en esfera) y el sombreado por normal de cara.
    pub fn demo_scene(dim: [u32; 3]) -> Self {
        let mut g = Self::new(dim);
        let [dx, dy, dz] = dim;

        // Piso: 2 capas grises abajo, con un leve damero para leer la perspectiva.
        for z in 0..dz {
            for x in 0..dx {
                let chk = ((x / 4 + z / 4) % 2) == 0;
                let base = if chk { 70 } else { 95 };
                for y in 0..2 {
                    g.set(x, y, z, [base, base + 8, base + 16]);
                }
            }
        }

        // Esfera centrada, color por posición normalizada.
        let cx = dx as f32 / 2.0;
        let cy = dy as f32 * 0.55;
        let cz = dz as f32 / 2.0;
        let r = (dx.min(dy).min(dz) as f32) * 0.3;
        for z in 0..dz {
            for y in 0..dy {
                for x in 0..dx {
                    let (fx, fy, fz) = (x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                    let d = ((fx - cx).powi(2) + (fy - cy).powi(2) + (fz - cz).powi(2)).sqrt();
                    if d <= r {
                        let rr = (fx / dx as f32 * 255.0) as u8;
                        let gg = (fy / dy as f32 * 255.0) as u8;
                        let bb = (fz / dz as f32 * 255.0) as u8;
                        g.set(x, y, z, [rr, gg, bb]);
                    }
                }
            }
        }
        g
    }
}
