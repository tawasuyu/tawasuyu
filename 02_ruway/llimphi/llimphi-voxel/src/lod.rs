//! LOD del horizonte: una **malla gruesa** del terreno circundante como telón de
//! fondo, para que más allá de la ventana voxel streameada se vean colinas
//! lejanas en vez de un muro de niebla. Es el híbrido clásico **voxel cerca /
//! malla-LOD lejos**: se compone con los voxels finos por el **depth compartido**
//! de [`Scene3d`](llimphi_3d::Scene3d) (los voxels ocluyen la malla donde se
//! solapan; afuera, la malla muestra el relieve distante).
//!
//! La malla se genera muestreando [`column_height`](crate::column_height) a paso
//! grueso (sin deps de render: el `Renderer3d` es flat-color, así que la luz y la
//! **niebla por distancia** se hornean en el color de cada vértice en CPU,
//! imitando la atmósfera del pase voxel para que el horizonte funda sin costura).

use llimphi_3d::Vertex3d;

use crate::terrain::column_height;

/// Mezcla lineal de dos colores RGB `[f32;3]`.
#[inline]
fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

#[inline]
fn srgb(c: [u8; 3]) -> [f32; 3] {
    [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]
}

/// Color del terreno (banda por altura) ya **sombreado** por `shade` (difuso) —
/// imita las bandas de `terrain.rs` para que la costura con los voxels finos no
/// salte. `fh` = altura normalizada; `is_water` pinta la superficie del mar.
fn band(fh: f32, is_water: bool) -> [f32; 3] {
    if is_water {
        return srgb([44, 96, 140]);
    }
    let rock = srgb([88, 86, 92]);
    let snow = srgb([236, 240, 250]);
    let grass_lo = srgb([54, 110, 52]);
    let grass_hi = srgb([96, 150, 70]);
    let sand = srgb([196, 182, 130]);
    if fh < 0.33 {
        sand
    } else if fh < 0.55 {
        mix3(grass_lo, grass_hi, (fh - 0.33) / 0.22)
    } else if fh < 0.72 {
        mix3(grass_hi, rock, (fh - 0.55) / 0.17)
    } else if fh < 0.82 {
        rock
    } else {
        mix3(rock, snow, (fh - 0.82) / 0.10)
    }
}

/// Parámetros de la falda LOD.
pub struct LodParams {
    /// Centro de la ventana en **mundo** `[wx, wz]` (la malla se centra ahí, igual
    /// que los voxels: posición renderizada = `mundo − centro`).
    pub center_xz: [i32; 2],
    /// Lado de la ventana voxel fina (voxels) — se deja un **hueco** ahí para que
    /// los voxels la llenen (la malla gruesa sólo rodea).
    pub window_xz: u32,
    /// Medio-alcance de la falda más allá del centro (voxels).
    pub span: i32,
    /// Paso de muestreo grueso (voxels). Mayor = más barato / más facetado.
    pub stride: i32,
    /// Color del horizonte (hacia el que funde la niebla) + densidad (espeja la
    /// `Atmosphere` del pase voxel).
    pub sky_horizon: [u8; 3],
    pub fog_density: f32,
    /// Dirección hacia el sol (para el sombreado difuso horneado).
    pub sun_dir: [f32; 3],
}

/// Genera la malla de la falda LOD para una ventana. Devuelve `(vértices,
/// índices u16)` listos para [`Renderer3d::set_geometry`](llimphi_3d::Renderer3d).
/// `dim`/`seed` definen el mismo terreno procedural que los voxels.
pub fn lod_skirt(p: &LodParams, dim: [u32; 3], seed: u32) -> (Vec<Vertex3d>, Vec<u16>) {
    let [cx, cz] = p.center_xz;
    let dy = dim[1] as f32;
    let sea = (dim[1] as f32 * 0.30) as i32;
    let stride = p.stride.max(1);
    let span = p.span.max(stride);
    let half = p.window_xz as i32 / 2;
    // Margen: solapar un poco el hueco con la ventana para que no quede una rendija
    // entre la malla y los voxels (el depth resuelve la oclusión del solape).
    let hole = (half - stride).max(0);

    // Altura renderizada de la columna de mundo (tierra o superficie del mar).
    let surf = |wx: i32, wz: i32| -> (i32, bool) {
        let h = column_height(wx, wz, dim, seed) as i32;
        if h < sea {
            (sea, true)
        } else {
            (h, false)
        }
    };

    let sun = {
        let s = p.sun_dir;
        let l = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt().max(1e-6);
        [s[0] / l, s[1] / l, s[2] / l]
    };
    let sky = srgb(p.sky_horizon);

    // Grilla de vértices [cx-span, cx+span] × [cz-span, cz+span] a paso `stride`.
    let n = ((2 * span) / stride) as usize + 1;
    let mut verts: Vec<Vertex3d> = Vec::with_capacity(n * n);
    let coord = |i: usize| -> i32 { -span + i as i32 * stride };

    for iz in 0..n {
        let wz = cz + coord(iz);
        for ix in 0..n {
            let wx = cx + coord(ix);
            let (h, water) = surf(wx, wz);
            // Normal por diferencias centrales del relieve (para el sombreado).
            let (hl, _) = surf(wx - stride, wz);
            let (hr, _) = surf(wx + stride, wz);
            let (hd, _) = surf(wx, wz - stride);
            let (hu, _) = surf(wx, wz + stride);
            let nx = (hl - hr) as f32;
            let nz = (hd - hu) as f32;
            let ny = 2.0 * stride as f32;
            let nl = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
            let ndl = ((nx * sun[0] + ny * sun[1] + nz * sun[2]) / nl).max(0.0);
            let shade = 0.45 + 0.6 * ndl; // ambiente + difuso (≈ pase voxel)

            let fh = h as f32 / dy;
            let mut color = band(fh, water);
            color = [color[0] * shade, color[1] * shade, color[2] * shade];

            // Niebla por distancia al centro (≈ cámara): funde al horizonte.
            let (dxf, dzf) = ((wx - cx) as f32, (wz - cz) as f32);
            let dist = (dxf * dxf + dzf * dzf).sqrt();
            // Cap < 1: el horizonte conserva algo de silueta/relieve (no se lava a
            // cielo puro), que es lo que hace legible que "el mundo sigue".
            let fog = (1.0 - (-dist * p.fog_density).exp()).min(0.9);
            color = mix3(color, sky, fog);

            verts.push(Vertex3d {
                pos: [(wx - cx) as f32, h as f32 - dy * 0.5, (wz - cz) as f32],
                color,
            });
        }
    }

    // Índices: dos triángulos por celda, salvo las celdas dentro del hueco central
    // (las llena la ventana voxel fina).
    let mut indices: Vec<u16> = Vec::new();
    for iz in 0..n - 1 {
        for ix in 0..n - 1 {
            // Centro de la celda en mundo, para el test del hueco.
            let mwx = coord(ix) + stride / 2;
            let mwz = coord(iz) + stride / 2;
            if mwx.abs() < hole && mwz.abs() < hole {
                continue; // dentro de la ventana fina → la llenan los voxels
            }
            let a = (iz * n + ix) as u16;
            let b = (iz * n + ix + 1) as u16;
            let c = ((iz + 1) * n + ix) as u16;
            let d = ((iz + 1) * n + ix + 1) as u16;
            // CCW vista desde arriba.
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    (verts, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skirt_genera_geometria_con_hueco() {
        let dim = [128, 56, 128];
        let p = LodParams {
            center_xz: [0, 0],
            window_xz: 128,
            span: 256,
            stride: 8,
            sky_horizon: [200, 216, 234],
            fog_density: 0.01,
            sun_dir: [0.5, 0.6, 0.3],
        };
        let (verts, indices) = lod_skirt(&p, dim, 1);
        assert!(!verts.is_empty() && !indices.is_empty());
        assert_eq!(indices.len() % 3, 0, "triángulos completos");
        assert!(*indices.iter().max().unwrap() < verts.len() as u16, "índices en rango");
        // Debe haber un hueco: menos triángulos que la grilla completa.
        let n = ((2 * p.span) / p.stride) as usize + 1;
        let full = (n - 1) * (n - 1) * 2;
        assert!(indices.len() / 3 < full, "el hueco recortó triángulos");
    }
}
