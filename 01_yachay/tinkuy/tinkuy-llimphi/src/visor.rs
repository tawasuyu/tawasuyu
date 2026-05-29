//! `visor` — proyección 3D→2D del visor MVP (E3).
//!
//! Sin cámara orbital ni perspectiva: una proyección **axonométrica fija**
//! con `z` empujando ligeramente hacia arriba-derecha. Suficiente para que
//! el lattice 4³ se vea como una caja en lugar de un punto plano.
//!
//! La proyección vive aislada acá (sin dependencias gráficas) para que sea
//! testeable de forma pura — el `paint_with` del `lib.rs` consume estas
//! funciones y agrega el resto (mapeo a canvas, fill/stroke, colorización).

/// Coeficiente de empuje horizontal de `z`. Eje `u` final: `x + z * TZX`.
pub const TZX: f32 = 0.6;

/// Coeficiente de empuje vertical de `z`. Eje `v` final: `y + z * TZY`.
pub const TZY: f32 = 0.4;

/// Proyecta un punto 3D a coordenadas planas `(u, v)` en el mismo
/// sistema "y arriba" que la simulación — el caller hace el flip y → canvas.
#[inline]
pub fn project(x: f32, y: f32, z: f32) -> (f32, f32) {
    (x + z * TZX, y + z * TZY)
}

/// Bounding box proyectada de los 8 corners de la caja sim. Devuelve
/// `(umin, umax, vmin, vmax)`.
pub fn project_bbox(bmin: [f32; 3], bmax: [f32; 3]) -> (f32, f32, f32, f32) {
    let mut umin = f32::INFINITY;
    let mut umax = f32::NEG_INFINITY;
    let mut vmin = f32::INFINITY;
    let mut vmax = f32::NEG_INFINITY;
    for &cx in &[bmin[0], bmax[0]] {
        for &cy in &[bmin[1], bmax[1]] {
            for &cz in &[bmin[2], bmax[2]] {
                let (u, v) = project(cx, cy, cz);
                if u < umin {
                    umin = u;
                }
                if u > umax {
                    umax = u;
                }
                if v < vmin {
                    vmin = v;
                }
                if v > vmax {
                    vmax = v;
                }
            }
        }
    }
    (umin, umax, vmin, vmax)
}

/// Las 12 aristas de la caja sim, expresadas como pares de índices sobre
/// el array de corners en el orden canónico de [`box_corners`].
pub const BOX_EDGES: [(usize, usize); 12] = [
    (0, 1), (1, 2), (2, 3), (3, 0), // piso z=zmin
    (4, 5), (5, 6), (6, 7), (7, 4), // techo z=zmax
    (0, 4), (1, 5), (2, 6), (3, 7), // verticales
];

/// Los 8 corners de la caja en orden canónico (ver [`BOX_EDGES`]):
/// 0..=3 piso (z=zmin) recorrido CCW, 4..=7 techo (z=zmax) en el mismo orden.
pub fn box_corners(bmin: [f32; 3], bmax: [f32; 3]) -> [(f32, f32, f32); 8] {
    [
        (bmin[0], bmin[1], bmin[2]),
        (bmax[0], bmin[1], bmin[2]),
        (bmax[0], bmax[1], bmin[2]),
        (bmin[0], bmax[1], bmin[2]),
        (bmin[0], bmin[1], bmax[2]),
        (bmax[0], bmin[1], bmax[2]),
        (bmax[0], bmax[1], bmax[2]),
        (bmin[0], bmax[1], bmax[2]),
    ]
}

/// Clave de orden "painter's algorithm": mayor → más al fondo, debe pintarse
/// primero. Mezcla `z` (depth principal) con un toque de `x` para que las
/// partículas alineadas en `z` no se solapen exactamente.
#[inline]
pub fn depth_key(x: f32, z: f32) -> f32 {
    z + x * 0.3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_eje_x_es_identidad_en_u() {
        let (u, v) = project(1.0, 0.0, 0.0);
        assert!((u - 1.0).abs() < 1e-6);
        assert!(v.abs() < 1e-6);
    }

    #[test]
    fn project_eje_y_es_identidad_en_v() {
        let (u, v) = project(0.0, 1.0, 0.0);
        assert!(u.abs() < 1e-6);
        assert!((v - 1.0).abs() < 1e-6);
    }

    #[test]
    fn project_eje_z_empuja_arriba_derecha() {
        let (u, v) = project(0.0, 0.0, 1.0);
        assert!((u - TZX).abs() < 1e-6);
        assert!((v - TZY).abs() < 1e-6);
    }

    #[test]
    fn bbox_caja_unidad_origen() {
        let (umin, umax, vmin, vmax) = project_bbox([0.0; 3], [1.0; 3]);
        assert!(umin.abs() < 1e-6);
        assert!((umax - (1.0 + TZX)).abs() < 1e-6);
        assert!(vmin.abs() < 1e-6);
        assert!((vmax - (1.0 + TZY)).abs() < 1e-6);
    }

    #[test]
    fn corners_y_aristas_consistentes() {
        let corners = box_corners([0.0; 3], [1.0; 3]);
        // Las 8 esquinas son distintas.
        for i in 0..8 {
            for j in (i + 1)..8 {
                assert!(corners[i] != corners[j], "corners {} y {} duplicados", i, j);
            }
        }
        // Las 12 aristas conectan corners válidos y cada par es único.
        for &(a, b) in &BOX_EDGES {
            assert!(a < 8 && b < 8 && a != b);
        }
    }

    #[test]
    fn depth_key_ordena_z_principal() {
        // Dos partículas con misma x: la de mayor z queda más al fondo.
        let k_near = depth_key(0.0, 0.0);
        let k_far = depth_key(0.0, 5.0);
        assert!(k_far > k_near);
        // Con misma z, x grande empuja un poquito más al fondo (estabilidad
        // de orden cuando las depths colapsan).
        let k_a = depth_key(0.0, 3.0);
        let k_b = depth_key(2.0, 3.0);
        assert!(k_b > k_a);
    }
}
