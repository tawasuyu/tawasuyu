//! `WorldStream` — ventana voxel acotada que se desliza por un mundo procedural
//! **ilimitado** (M6: streaming). En vez de un grid fijo centrado en el origen,
//! mantenemos una ventana de `dim` voxels cuya esquina cae en una columna de
//! **mundo** `origin`; al caminar, [`WorldStream::follow`] reubica la ventana
//! (en pasos de `step`) y regenera el terreno con [`fill_terrain_window`]. Como
//! el relieve es función pura de mundo ([`column_height`]), las costuras encajan
//! y se puede caminar indefinidamente sin "muro" ni repetición.
//!
//! **Alcance (MVP):** la regeneración es de la ventana **entera** por cada paso
//! cruzado (no un *shift* parcial que sólo rellene el borde nuevo). Es O(ventana)
//! por reubicación, no por frame — sólo al cruzar un múltiplo de `step`. El grid
//! queda **dirty completo** para que `VoxelRenderer::sync` re-suba (o se
//! reconstruya el renderer). El *shift* incremental (copiar la zona común y
//! generar sólo la franja nueva) y el LOD del horizonte quedan como optimización
//! futura — ver `MOTOR-VOXEL.md` §7.

use llimphi_3d::VoxelGrid;

use crate::terrain::fill_terrain_window;

/// Ventana de mundo que sigue a un foco (la cámara/jugador) por un terreno
/// procedural ilimitado.
pub struct WorldStream {
    grid: VoxelGrid,
    dim: [u32; 3],
    seed: u32,
    /// Columna de mundo `[wx, wz]` donde cae la esquina local `(0,0)`.
    origin: [i32; 2],
    /// Granularidad de reubicación (voxels). Recentrar sólo cuando el foco se
    /// aleja del centro más de medio paso evita regenerar cada voxel caminado.
    step: i32,
}

impl WorldStream {
    /// Crea una ventana de `dim` centrada en la columna de mundo `(center_x,
    /// center_z)` y la genera. `step` = granularidad de reubicación en voxels
    /// (típicamente el lado de un *brick*, p.ej. 8).
    pub fn new(dim: [u32; 3], seed: u32, center_x: i32, center_z: i32, step: u32) -> Self {
        let step = step.max(1) as i32;
        let origin = Self::origin_for(dim, center_x, center_z, step);
        let mut grid = VoxelGrid::new(dim);
        fill_terrain_window(&mut grid, origin, seed);
        Self { grid, dim, seed, origin, step }
    }

    /// Origen (esquina) de ventana que centra la columna de mundo `(cx, cz)`,
    /// **snappeado** a `step` para que sólo cambie a saltos discretos.
    fn origin_for(dim: [u32; 3], cx: i32, cz: i32, step: i32) -> [i32; 2] {
        let half_x = dim[0] as i32 / 2;
        let half_z = dim[2] as i32 / 2;
        [
            snap(cx - half_x, step),
            snap(cz - half_z, step),
        ]
    }

    /// Reubica la ventana para centrar la columna de mundo `(cx, cz)` y, si el
    /// origen snappeado cambió, **regenera** el terreno. Devuelve `true` si hubo
    /// regeneración (el grid quedó dirty → el caller debe `sync`/reconstruir).
    pub fn follow(&mut self, cx: i32, cz: i32) -> bool {
        let want = Self::origin_for(self.dim, cx, cz, self.step);
        if want == self.origin {
            return false;
        }
        self.origin = want;
        fill_terrain_window(&mut self.grid, want, self.seed);
        true
    }

    /// Grid actual (para renderizar / `VoxelRenderer::new`).
    pub fn grid(&self) -> &VoxelGrid {
        &self.grid
    }

    /// Grid mutable (para `VoxelRenderer::sync`, que toma `&mut`).
    pub fn grid_mut(&mut self) -> &mut VoxelGrid {
        &mut self.grid
    }

    /// Columna de mundo de la esquina local `(0,0)`.
    pub fn origin(&self) -> [i32; 2] {
        self.origin
    }

    /// Mapea una columna de **mundo** a coordenada **local** de ventana, o `None`
    /// si cae afuera. Para ubicar al jugador/entidades dentro del grid actual.
    pub fn world_to_local(&self, wx: i32, wz: i32) -> Option<(u32, u32)> {
        let lx = wx - self.origin[0];
        let lz = wz - self.origin[1];
        if lx >= 0 && lz >= 0 && lx < self.dim[0] as i32 && lz < self.dim[2] as i32 {
            Some((lx as u32, lz as u32))
        } else {
            None
        }
    }

    /// Mapea una coordenada **local** de ventana a su columna de **mundo**.
    pub fn local_to_world(&self, lx: u32, lz: u32) -> (i32, i32) {
        (self.origin[0] + lx as i32, self.origin[1] + lz as i32)
    }
}

/// Redondea `v` al múltiplo de `step` más cercano hacia abajo (floor con signo).
#[inline]
fn snap(v: i32, step: i32) -> i32 {
    (v.div_euclid(step)) * step
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_redondea_con_signo() {
        assert_eq!(snap(0, 8), 0);
        assert_eq!(snap(7, 8), 0);
        assert_eq!(snap(8, 8), 8);
        assert_eq!(snap(-1, 8), -8);
        assert_eq!(snap(-8, 8), -8);
        assert_eq!(snap(-9, 8), -16);
    }

    /// Caminar menos de un paso NO regenera; cruzar un paso SÍ.
    #[test]
    fn follow_regenera_solo_al_cruzar_un_paso() {
        let dim = [48, 32, 48];
        let mut s = WorldStream::new(dim, 9, 0, 0, 8);
        let o0 = s.origin();
        // Moverse dentro del mismo bloque de 8: sin regen.
        assert!(!s.follow(3, 2));
        assert_eq!(s.origin(), o0);
        // Cruzar varios pasos: regen y el origen siguió al foco.
        assert!(s.follow(64, 0));
        assert_ne!(s.origin(), o0);
        // El centro de ventana quedó cerca del foco (dentro de un paso).
        let cx = s.origin()[0] + dim[0] as i32 / 2;
        assert!((cx - 64).abs() <= 8, "ventana centrada en el foco (cx={cx})");
    }

    /// Tras seguir el foco, el contenido coincide con generar esa ventana de cero
    /// (la regeneración es completa y determinista por origen).
    #[test]
    fn contenido_tras_follow_igual_a_generacion_directa() {
        let dim = [40, 28, 40];
        let seed = 555;
        let mut s = WorldStream::new(dim, seed, 0, 0, 8);
        s.follow(120, -80);
        let origin = s.origin();

        let mut directo = VoxelGrid::new(dim);
        fill_terrain_window(&mut directo, origin, seed);

        for z in 0..dim[2] {
            for y in 0..dim[1] {
                for x in 0..dim[0] {
                    assert_eq!(
                        s.grid().get(x, y, z),
                        directo.get(x, y, z),
                        "discrepa en ({x},{y},{z})"
                    );
                }
            }
        }
    }
}
