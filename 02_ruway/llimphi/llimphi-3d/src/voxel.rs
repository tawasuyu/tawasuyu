//! `VoxelGrid` — grid de voxels denso y acotado (CPU side). Cada voxel es
//! RGBA8: `rgb` = color, `a` = ocupación (`0` vacío, `>0` sólido). Se sube a
//! una textura 3D de GPU que el shader ray-march recorre por DDA.
//!
//! M1 es **denso** a propósito (lo más simple que funciona). El salto a sparse
//! (SVO/brickmap, saltar el aire) es M2 — ver `MOTOR-VOXEL.md` §11.2.
//!
//! M3 agrega **dirty tracking**: cada `set`/`clear` expande una caja AABB de la
//! región cambiada. `VoxelRenderer::sync` sube sólo esa sub-caja (fina + bricks
//! gruesos afectados) — la actualización incremental que reemplaza al re-mesh.

/// Caja AABB de voxels cambiados desde el último `take_dirty`: `[xmin, ymin,
/// zmin, xmax, ymax, zmax]` inclusiva.
pub type DirtyBox = [u32; 6];

/// Grid denso de voxels RGBA8. Índice lineal `x + y*dx + z*dx*dy` (x contiguo),
/// que es justo el layout que espera `queue.write_texture` (filas en x, luego y,
/// luego capas en z).
#[derive(Clone)]
pub struct VoxelGrid {
    dim: [u32; 3],
    data: Vec<[u8; 4]>,
    /// AABB de voxels mutados desde el último `take_dirty`. `None` = sin cambios.
    dirty: Option<DirtyBox>,
}

impl VoxelGrid {
    /// Grid vacío de `dim = [dx, dy, dz]` voxels.
    pub fn new(dim: [u32; 3]) -> Self {
        let n = (dim[0] * dim[1] * dim[2]) as usize;
        Self {
            dim,
            data: vec![[0, 0, 0, 0]; n],
            dirty: None,
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

    #[inline]
    fn mark_dirty(&mut self, x: u32, y: u32, z: u32) {
        match &mut self.dirty {
            None => self.dirty = Some([x, y, z, x, y, z]),
            Some(d) => {
                d[0] = d[0].min(x);
                d[1] = d[1].min(y);
                d[2] = d[2].min(z);
                d[3] = d[3].max(x);
                d[4] = d[4].max(y);
                d[5] = d[5].max(z);
            }
        }
    }

    /// Toma y limpia la caja de cambios pendientes. `VoxelRenderer::sync` la usa
    /// para subir sólo lo mutado. `None` si no hubo cambios desde la última toma.
    pub fn take_dirty(&mut self) -> Option<DirtyBox> {
        self.dirty.take()
    }

    /// Descarta los cambios pendientes sin subirlos (tras un upload completo, el
    /// estado inicial ya está en GPU).
    pub fn reset_dirty(&mut self) {
        self.dirty = None;
    }

    /// Marca un voxel sólido con color `rgb` (alpha = 255). Fuera de rango: no-op.
    pub fn set(&mut self, x: u32, y: u32, z: u32, rgb: [u8; 3]) {
        if x < self.dim[0] && y < self.dim[1] && z < self.dim[2] {
            let i = self.idx(x, y, z);
            self.data[i] = [rgb[0], rgb[1], rgb[2], 255];
            self.mark_dirty(x, y, z);
        }
    }

    /// Vacía un voxel.
    pub fn clear(&mut self, x: u32, y: u32, z: u32) {
        if x < self.dim[0] && y < self.dim[1] && z < self.dim[2] {
            let i = self.idx(x, y, z);
            self.data[i] = [0, 0, 0, 0];
            self.mark_dirty(x, y, z);
        }
    }

    #[inline]
    fn solid(&self, x: u32, y: u32, z: u32) -> bool {
        self.data[self.idx(x, y, z)][3] > 0
    }

    /// Mapa de ocupación grueso por *bricks* de `brick³` voxels (M2): un texel
    /// por brick, `255` si el brick contiene algún voxel sólido, `0` si está
    /// todo vacío. El shader marcha primero esta grilla gruesa y se salta los
    /// bricks vacíos enteros en un paso (empty-space skipping). Devuelve
    /// `(dim_grueso, bytes R8)` con índice `cx + cy*cdx + cz*cdx*cdy`.
    pub fn coarse_occupancy(&self, brick: u32) -> ([u32; 3], Vec<u8>) {
        let b = brick.max(1);
        let cdim = [
            self.dim[0].div_ceil(b),
            self.dim[1].div_ceil(b),
            self.dim[2].div_ceil(b),
        ];
        let mut out = vec![0u8; (cdim[0] * cdim[1] * cdim[2]) as usize];
        for z in 0..self.dim[2] {
            for y in 0..self.dim[1] {
                for x in 0..self.dim[0] {
                    if self.solid(x, y, z) {
                        let (cx, cy, cz) = (x / b, y / b, z / b);
                        out[(cx + cy * cdim[0] + cz * cdim[0] * cdim[1]) as usize] = 255;
                    }
                }
            }
        }
        (cdim, out)
    }

    /// `255` si el brick `(cx,cy,cz)` (tamaño `b`) tiene algún voxel sólido,
    /// `0` si está todo vacío. Lo usa el brick pool para decidir si un brick
    /// necesita slot.
    pub fn brick_occupied(&self, b: u32, cx: u32, cy: u32, cz: u32) -> u8 {
        let (x0, y0, z0) = (cx * b, cy * b, cz * b);
        for z in z0..(z0 + b).min(self.dim[2]) {
            for y in y0..(y0 + b).min(self.dim[1]) {
                for x in x0..(x0 + b).min(self.dim[0]) {
                    if self.solid(x, y, z) {
                        return 255;
                    }
                }
            }
        }
        0
    }

    /// Extrae los voxels de un brick `(cx,cy,cz)` de lado `brick` como RGBA
    /// plano (`brick³` voxels, x contiguo), padeando con vacío los voxels fuera
    /// del grid (bricks de borde cuando `dim` no es múltiplo de `brick`). Es la
    /// unidad de subida al *pool* sparse (un slot del atlas = un brick).
    pub fn extract_brick(&self, brick: u32, cx: u32, cy: u32, cz: u32) -> Vec<u8> {
        let b = brick;
        let mut out = vec![0u8; (b * b * b * 4) as usize];
        for lz in 0..b {
            for ly in 0..b {
                for lx in 0..b {
                    let (x, y, z) = (cx * b + lx, cy * b + ly, cz * b + lz);
                    if x < self.dim[0] && y < self.dim[1] && z < self.dim[2] {
                        let px = self.data[self.idx(x, y, z)];
                        let o = ((lx + ly * b + lz * b * b) * 4) as usize;
                        out[o..o + 4].copy_from_slice(&px);
                    }
                }
            }
        }
        out
    }

    /// Extrae una sub-caja RGBA contigua `[origin, origin+ext)` para subirla con
    /// `queue.write_texture` (M3: upload incremental de la región fina mutada).
    pub fn extract_fine(&self, origin: [u32; 3], ext: [u32; 3]) -> Vec<u8> {
        let mut out = Vec::with_capacity((ext[0] * ext[1] * ext[2] * 4) as usize);
        for z in origin[2]..origin[2] + ext[2] {
            for y in origin[1]..origin[1] + ext[1] {
                let row = self.idx(origin[0], y, z);
                for i in 0..ext[0] as usize {
                    out.extend_from_slice(&self.data[row + i]);
                }
            }
        }
        out
    }

    /// Recalcula la ocupación gruesa de la caja de bricks `[cmin, cmin+cext)` y
    /// la devuelve contigua (R8) para subir sólo esos bricks (M3).
    pub fn coarse_region(&self, brick: u32, cmin: [u32; 3], cext: [u32; 3]) -> Vec<u8> {
        let b = brick.max(1);
        let mut out = Vec::with_capacity((cext[0] * cext[1] * cext[2]) as usize);
        for cz in cmin[2]..cmin[2] + cext[2] {
            for cy in cmin[1]..cmin[1] + cext[1] {
                for cx in cmin[0]..cmin[0] + cext[0] {
                    out.push(self.brick_occupied(b, cx, cy, cz));
                }
            }
        }
        out
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

        // Pilares: dan rincones para el AO y proyectan/reciben sombras.
        let pillars: [(u32, u32, u32, [u8; 3]); 3] = [
            (dx / 5, dz / 4, dy * 7 / 10, [200, 120, 90]),
            (dx * 4 / 5, dz / 3, dy / 2, [110, 170, 120]),
            (dx / 3, dz * 4 / 5, dy * 3 / 5, [120, 130, 210]),
        ];
        for (px, pz, ph, col) in pillars {
            for y in 2..(2 + ph).min(dy) {
                for dxx in 0..3u32 {
                    for dzz in 0..3u32 {
                        g.set(px + dxx, y, pz + dzz, col);
                    }
                }
            }
        }
        // Estado inicial: el upload completo lo cubre, no es "mutación".
        g.reset_dirty();
        g
    }
}
