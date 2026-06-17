//! Picking/edición por **raycast de voxels** — la mecánica núcleo de un juego
//! voxel (mirar → bloque). DDA Amanatides-Woo en CPU sobre un [`VoxelGrid`],
//! espejo del traversal que hace el shader. Devuelve el voxel sólido golpeado,
//! la cara de entrada (normal) y la celda vacía adyacente donde *colocar*.
//!
//! Coordenadas en **espacio de grilla** (voxel = 1, el mundo ocupa `[0, dim]`).
//! El motor centra la grilla en el origen, así que para tirar el rayo desde la
//! cámara: `origin_grilla = eye_mundo + dim/2`. [`raycast`] no toca la GPU;
//! editar = `grid.set/clear` + `VoxelRenderer::sync` (subida incremental).

use llimphi_3d::VoxelGrid;

/// Resultado de un [`raycast`] que pegó en un voxel sólido.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoxelHit {
    /// Voxel sólido golpeado (coordenadas de grilla).
    pub cell: [i32; 3],
    /// Normal de la cara de entrada (uno de ±X/±Y/±Z), apuntando al aire desde
    /// donde vino el rayo. `[0,0,0]` si el origen ya estaba dentro de un sólido.
    pub normal: [i32; 3],
    /// Celda vacía adyacente (`cell + normal`): dónde **colocaría** un bloque
    /// nuevo un click de "construir".
    pub place: [i32; 3],
    /// Distancia (en unidades de voxel) del origen al impacto.
    pub dist: f32,
}

/// Marcha un rayo `origin + t·dir` por la grilla hasta el primer voxel sólido,
/// hasta `max_dist` unidades. `dir` no necesita estar normalizado (se normaliza
/// acá, y `dist` queda en unidades de voxel). `None` si no pega nada.
pub fn raycast(grid: &VoxelGrid, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> Option<VoxelHit> {
    // Normalizar para que t sea distancia real.
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len < 1e-9 {
        return None;
    }
    let d = [dir[0] / len, dir[1] / len, dir[2] / len];

    // Celda inicial y parámetros DDA por eje.
    let mut cell = [
        origin[0].floor() as i32,
        origin[1].floor() as i32,
        origin[2].floor() as i32,
    ];
    let mut step = [0i32; 3];
    let mut t_max = [f32::INFINITY; 3];
    let mut t_delta = [f32::INFINITY; 3];
    for a in 0..3 {
        if d[a] > 1e-9 {
            step[a] = 1;
            t_delta[a] = 1.0 / d[a];
            t_max[a] = (cell[a] as f32 + 1.0 - origin[a]) / d[a];
        } else if d[a] < -1e-9 {
            step[a] = -1;
            t_delta[a] = -1.0 / d[a];
            t_max[a] = (cell[a] as f32 - origin[a]) / d[a];
        }
    }

    // ¿El origen ya está dentro de un sólido? (cavar desde adentro)
    if grid.is_solid(cell[0], cell[1], cell[2]) {
        return Some(VoxelHit { cell, normal: [0, 0, 0], place: cell, dist: 0.0 });
    }

    let mut t = 0.0f32;
    let mut normal = [0i32; 3];
    // Tope de pasos generoso (suma de extensiones + margen) para no colgar.
    let dim = grid.dim();
    let max_steps = (dim[0] + dim[1] + dim[2]) as i32 * 2 + 8;
    for _ in 0..max_steps {
        // Avanzar al siguiente plano de voxel (eje con menor t_max).
        let axis = if t_max[0] < t_max[1] && t_max[0] < t_max[2] {
            0
        } else if t_max[1] < t_max[2] {
            1
        } else {
            2
        };
        t = t_max[axis];
        if t > max_dist {
            return None;
        }
        cell[axis] += step[axis];
        t_max[axis] += t_delta[axis];
        normal = [0, 0, 0];
        normal[axis] = -step[axis]; // cara por la que entramos

        if grid.is_solid(cell[0], cell[1], cell[2]) {
            return Some(VoxelHit {
                cell,
                normal,
                place: [cell[0] + normal[0], cell[1] + normal[1], cell[2] + normal[2]],
                dist: t,
            });
        }
    }
    let _ = normal;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pega_un_voxel_solo_y_da_cara_y_place() {
        // Grid 8³ con un único voxel sólido en (4,4,4).
        let mut g = VoxelGrid::new([8, 8, 8]);
        g.set(4, 4, 4, [200, 50, 50]);

        // Rayo desde -X hacia +X a la altura/profundidad del voxel.
        let hit = raycast(&g, [0.5, 4.5, 4.5], [1.0, 0.0, 0.0], 100.0).expect("debe pegar");
        assert_eq!(hit.cell, [4, 4, 4]);
        assert_eq!(hit.normal, [-1, 0, 0]); // entró por la cara -X
        assert_eq!(hit.place, [3, 4, 4]); // colocar queda un voxel antes
    }

    #[test]
    fn no_pega_aire() {
        let g = VoxelGrid::new([8, 8, 8]);
        assert!(raycast(&g, [0.5, 0.5, 0.5], [1.0, 0.0, 0.0], 100.0).is_none());
    }

    #[test]
    fn respeta_max_dist() {
        let mut g = VoxelGrid::new([64, 8, 8]);
        g.set(60, 4, 4, [10, 10, 10]);
        // El voxel está a ~55 unidades; con max_dist 10 no debe alcanzarlo.
        assert!(raycast(&g, [4.5, 4.5, 4.5], [1.0, 0.0, 0.0], 10.0).is_none());
        assert!(raycast(&g, [4.5, 4.5, 4.5], [1.0, 0.0, 0.0], 100.0).is_some());
    }
}
