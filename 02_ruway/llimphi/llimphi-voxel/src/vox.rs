//! Importación de modelos **MagicaVoxel `.vox`** al motor: convierte un
//! [`foreign_vox::VoxModel`] (neutral, leído por el puente) en un
//! [`VoxelGrid`](llimphi_3d::VoxelGrid) del motor, para meter **sets y
//! personajes diseñados afuera** a una escena/película voxel.
//!
//! Ejes: MagicaVoxel usa `z` como **arriba**; el motor usa `y` arriba. La
//! conversión mapea `vox (x, y, z) → grid (x, z, y)` (la `z` del `.vox` sube a la
//! `y` del grid, la `y` del `.vox` pasa a la profundidad `z`).
//!
//! La capa de juego es la dueña de esto (no el motor ni el puente): el puente
//! sólo entiende bytes, el motor sólo voxels; acá se casan (CLAUDE.md regla #4).

use std::fmt;
use std::path::Path;

use foreign_vox::{VoxError, VoxModel};
use llimphi_3d::VoxelGrid;

/// Error al cargar un `.vox` desde disco.
#[derive(Debug)]
pub enum VoxLoadError {
    /// Falló la lectura del archivo.
    Io(std::io::Error),
    /// El contenido no es un `.vox` válido.
    Parse(VoxError),
}

impl fmt::Display for VoxLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VoxLoadError::Io(e) => write!(f, "leyendo .vox: {e}"),
            VoxLoadError::Parse(e) => write!(f, "parseando .vox: {e}"),
        }
    }
}

impl std::error::Error for VoxLoadError {}

/// Convierte un `VoxModel` en un `VoxelGrid` ajustado a su tamaño (con los ejes
/// remapeados a la convención del motor). El estado dirty queda limpio (el grid
/// recién hecho se sube entero).
pub fn model_to_grid(m: &VoxModel) -> VoxelGrid {
    // dim del grid = [x, z(vox→y), y(vox→z)], mínimo 1 por eje.
    let dim = [m.size[0].max(1), m.size[2].max(1), m.size[1].max(1)];
    let mut g = VoxelGrid::new(dim);
    stamp(&mut g, m, [0, 0, 0]);
    g.reset_dirty();
    g
}

/// **Estampa** los voxels de un modelo dentro de un grid existente, con la
/// esquina del modelo en `origin` (espacio de grilla del motor). Para componer
/// *sets*: meter varias piezas `.vox` en un mismo mundo. Voxels transparentes
/// (`alpha 0`) se omiten; los que caen fuera del grid, también (`set` los ignora).
pub fn stamp(grid: &mut VoxelGrid, m: &VoxModel, origin: [u32; 3]) {
    for v in &m.voxels {
        let c = m.color(v);
        if c[3] == 0 {
            continue;
        }
        let gx = origin[0] + v.x as u32;
        let gy = origin[1] + v.z as u32; // z-arriba (vox) → y-arriba (grid)
        let gz = origin[2] + v.y as u32;
        grid.set(gx, gy, gz, [c[0], c[1], c[2]]);
    }
}

/// Carga el **primer** modelo de un archivo `.vox` como `VoxelGrid`.
pub fn load_grid(path: impl AsRef<Path>) -> Result<VoxelGrid, VoxLoadError> {
    let bytes = std::fs::read(path).map_err(VoxLoadError::Io)?;
    let models = foreign_vox::parse(&bytes).map_err(VoxLoadError::Parse)?;
    // `parse` ya garantiza ≥1 modelo (si no, devuelve NoModel).
    Ok(model_to_grid(&models[0]))
}

/// Compone una **escena multi-modelo** (`nTRN/nGRP/nSHP`) en un único `VoxelGrid`,
/// colocando cada modelo en su traslación de mundo resuelta por el grafo. En
/// MagicaVoxel la traslación `_t` ubica el **centro** del modelo, así que la esquina
/// va en `t − size/2`. El grid se dimensiona a la caja envolvente de todas las piezas
/// y se desplaza para que el mínimo caiga en el origen. Si la escena no trae grafo
/// (`.vox` viejo de un solo modelo), cae a [`model_to_grid`] del primer modelo.
pub fn scene_to_grid(scene: &foreign_vox::Scene) -> VoxelGrid {
    if scene.placements.is_empty() || scene.models.is_empty() {
        return model_to_grid(&scene.models[0]);
    }
    // Esquina (espacio grid: [x, z, y]) y tamaño grid de cada colocación.
    let mut corners: Vec<([i32; 3], [i32; 3])> = Vec::with_capacity(scene.placements.len());
    for p in &scene.placements {
        let m = &scene.models[p.model];
        let sz_grid = [m.size[0] as i32, m.size[2] as i32, m.size[1] as i32];
        let t = p.translation; // vox (z-arriba)
        // Centro → esquina (vox), luego remapeo de ejes a grid.
        let corner_vox = [t[0] - m.size[0] as i32 / 2, t[1] - m.size[1] as i32 / 2, t[2] - m.size[2] as i32 / 2];
        let corner_grid = [corner_vox[0], corner_vox[2], corner_vox[1]];
        corners.push((corner_grid, sz_grid));
    }
    // Caja envolvente.
    let mut lo = [i32::MAX; 3];
    let mut hi = [i32::MIN; 3];
    for (c, s) in &corners {
        for a in 0..3 {
            lo[a] = lo[a].min(c[a]);
            hi[a] = hi[a].max(c[a] + s[a]);
        }
    }
    let dim = [
        (hi[0] - lo[0]).max(1) as u32,
        (hi[1] - lo[1]).max(1) as u32,
        (hi[2] - lo[2]).max(1) as u32,
    ];
    let mut g = VoxelGrid::new(dim);
    for (p, (c, _)) in scene.placements.iter().zip(&corners) {
        let origin = [(c[0] - lo[0]) as u32, (c[1] - lo[1]) as u32, (c[2] - lo[2]) as u32];
        stamp(&mut g, &scene.models[p.model], origin);
    }
    g.reset_dirty();
    g
}

/// Carga una **escena** `.vox` completa (multi-modelo con grafo) como un único
/// `VoxelGrid` compuesto. Para un `.vox` de un solo modelo equivale a [`load_grid`].
pub fn load_scene_grid(path: impl AsRef<Path>) -> Result<VoxelGrid, VoxLoadError> {
    let bytes = std::fs::read(path).map_err(VoxLoadError::Io)?;
    let scene = foreign_vox::parse_scene(&bytes).map_err(VoxLoadError::Parse)?;
    Ok(scene_to_grid(&scene))
}

#[cfg(test)]
mod tests {
    use super::*;
    use foreign_vox::Voxel;

    #[test]
    fn remapea_ejes_z_arriba_a_y_arriba() {
        let mut m = VoxModel::new([2, 3, 4]); // x=2, y=3, z=4 (z arriba)
        m.palette[1] = [10, 20, 30, 255];
        // Voxel en vox (1, 2, 3) → grid (1, 3, 2).
        m.voxels = vec![Voxel { x: 1, y: 2, z: 3, i: 1 }];
        let g = model_to_grid(&m);
        assert_eq!(g.dim(), [2, 4, 3], "dim = [x, z, y]");
        assert!(g.is_solid(1, 3, 2), "vox(x,y,z) → grid(x,z,y)");
        assert_eq!(g.get(1, 3, 2), Some([10, 20, 30, 255]));
    }

    #[test]
    fn omite_transparentes() {
        let mut m = VoxModel::new([2, 2, 2]);
        m.palette[1] = [0, 0, 0, 0]; // transparente
        m.voxels = vec![Voxel { x: 0, y: 0, z: 0, i: 1 }];
        let g = model_to_grid(&m);
        assert!(!g.is_solid(0, 0, 0));
    }

    #[test]
    fn escena_compone_dos_modelos_con_offset() {
        use foreign_vox::{Placement, Scene};
        // Dos cubitos 2³ con un voxel en su esquina; el 2º trasladado +10 en x (vox).
        let cubo = || {
            let mut m = VoxModel::new([2, 2, 2]);
            m.palette[1] = [50, 60, 70, 255];
            m.voxels = vec![Voxel { x: 0, y: 0, z: 0, i: 1 }];
            m
        };
        let scene = Scene {
            models: vec![cubo(), cubo()],
            placements: vec![
                Placement { model: 0, translation: [0, 0, 0] },
                Placement { model: 1, translation: [10, 0, 0] },
            ],
        };
        let g = scene_to_grid(&scene);
        // Ambos modelos presentes y separados por ~10 en x (centro→esquina = −1 cada uno,
        // misma resta, así que la separación entre voxels se conserva = 10).
        let (dx, dy, dz) = (g.dim()[0] as i32, g.dim()[1] as i32, g.dim()[2] as i32);
        let solid: Vec<i32> =
            (0..dx).filter(|&x| (0..dy).any(|y| (0..dz).any(|z| g.is_solid(x, y, z)))).collect();
        assert_eq!(solid.len(), 2, "dos columnas sólidas separadas");
        assert_eq!(solid[1] - solid[0], 10, "separación de mundo preservada");
    }
}
