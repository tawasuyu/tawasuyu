//! World-gen procedural: terreno por ruido fractal (fbm propio, sin deps) →
//! [`VoxelGrid`] coloreado por bandas de altura/pendiente (agua, arena, pasto,
//! roca, nieve) con árboles. Contenido reusable por cualquier app/juego voxel.
//!
//! El terreno se define como **función pura de coordenadas de mundo** ([`column_height`]):
//! el mismo punto `(wx, wz)` da siempre el mismo relieve, sin importar en qué
//! ventana caiga. Eso es lo que hace posible el *streaming* (M6): mover una
//! ventana acotada por un mundo ilimitado y que las costuras encajen
//! ([`fill_terrain_window`] + [`WorldStream`](crate::WorldStream)).

use llimphi_3d::VoxelGrid;

/// Hash entero → `f32` en `[0, 1)`. Mezcla estilo PCG/xxhash chico, determinista.
/// Funciona con coordenadas negativas (`as u32` envuelve de forma estable).
#[inline]
pub(crate) fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = seed
        .wrapping_add((x as u32).wrapping_mul(0x9E37_79B9))
        .wrapping_add((y as u32).wrapping_mul(0x85EB_CA77));
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h & 0x00FF_FFFF) as f32 / 0x0100_0000 as f32
}

/// Suaviza `t` con la curva quíntica de Perlin (`6t⁵−15t⁴+10t³`).
#[inline]
pub(crate) fn smooth(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Ruido de valor bilineal en `(x, y)` continuos sobre la lattice entera.
/// `floor` maneja negativos (continuidad sobre coordenadas de mundo con signo).
fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let xf = smooth(x - xi as f32);
    let yf = smooth(y - yi as f32);
    let a = hash2(xi, yi, seed);
    let b = hash2(xi + 1, yi, seed);
    let c = hash2(xi, yi + 1, seed);
    let d = hash2(xi + 1, yi + 1, seed);
    let top = a + (b - a) * xf;
    let bot = c + (d - c) * xf;
    top + (bot - top) * yf
}

/// Fractional Brownian motion: suma de octavas de `value_noise` (frecuencia ×2,
/// amplitud ×`gain` por octava). Devuelve `[0, 1]` aprox (normalizado).
pub(crate) fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let mut freq = 1.0;
    let mut amp = 1.0;
    let mut sum = 0.0;
    let mut norm = 0.0;
    for o in 0..octaves {
        sum += value_noise(x * freq, y * freq, seed.wrapping_add(o.wrapping_mul(7919))) * amp;
        norm += amp;
        freq *= 2.0;
        amp *= 0.5;
    }
    sum / norm
}

/// Mezcla lineal de dos colores RGB.
#[inline]
fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * t) as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * t) as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * t) as u8,
    ]
}

/// Frecuencia espacial del relieve: ~4 colinas grandes a lo ancho de una ventana
/// de lado `span`. Es función del **tamaño de ventana** (constante mientras la
/// ventana no cambie de tamaño), así dos ventanas del mismo `dim` comparten la
/// misma escala → el streaming encaja. `min_h`/`amp` salen del alto del mundo.
#[inline]
pub(crate) fn world_scale(dim: [u32; 3]) -> f32 {
    4.0 / dim[0].max(dim[2]) as f32
}

/// Nivel del mar (índice `y`) para un mundo de alto `dy`.
#[inline]
fn sea_level(dy: u32) -> u32 {
    (dy as f32 * 0.30) as u32
}

/// Altura del terreno (índice `y` del voxel sólido superior) en la columna de
/// **mundo** `(wx, wz)`, para un mundo de dimensiones `dim` y `seed`. Es una
/// **función pura**: el mismo punto da la misma altura en cualquier ventana —
/// la clave de la continuidad del streaming. Combina un fbm base estirado
/// (océanos↔picos) con un término *ridged* sólo en lo alto (crestas afiladas).
pub fn column_height(wx: i32, wz: i32, dim: [u32; 3], seed: u32) -> u32 {
    let dy = dim[1];
    let scale = world_scale(dim);
    let min_h = (dy as f32 * 0.03) as u32;
    let amp = dy as f32 * 0.95;

    let c = fbm(wx as f32 * scale, wz as f32 * scale, 6, seed);
    let e0 = ((c - 0.35) * 2.0).clamp(0.0, 1.0);
    let e = e0 * e0 * (3.0 - 2.0 * e0); // smoothstep → mesetas + valles
    let ridge =
        1.0 - (fbm(wx as f32 * scale * 2.3, wz as f32 * scale * 2.3, 5, seed ^ 99) - 0.5).abs() * 2.0;
    let e = (e + e * e * ridge * 0.55).min(1.0);
    (min_h + (e * amp) as u32).min(dy - 1)
}

/// Padding (en voxels) alrededor de la ventana para precomputar alturas: cubre
/// el cálculo de pendiente (±1) y la copa de los árboles rooteados afuera (±2).
const PAD: i32 = 3;

/// Rellena `g` con el paisaje voxel cuya esquina local `(0,0)` cae en la columna
/// de **mundo** `origin = [wx, wz]`. Vacía el grid primero (`clear_all`) y lo
/// deja **dirty** para que `VoxelRenderer::sync` re-suba (o reconstruir el
/// renderer). Es la primitiva del streaming: dos ventanas contiguas encajan
/// porque todo sale de [`column_height`] (función de mundo).
///
/// `terrain(dim, seed)` es el caso `origin = [0, 0]` con el dirty reseteado.
pub fn fill_terrain_window(g: &mut VoxelGrid, origin: [i32; 2], seed: u32) {
    let dim = g.dim();
    let [dx, dy, dz] = dim;
    let sea = sea_level(dy);
    let (ox, oz) = (origin[0], origin[1]);

    g.clear_all();

    let rock = [88, 86, 92];
    let snow = [236, 240, 250];
    let grass_lo = [54, 110, 52];
    let grass_hi = [96, 150, 70];
    let sand = [196, 182, 130];
    let deep = [22, 52, 96];
    let shallow = [44, 110, 150];

    // Heightmap precomputado sobre la ventana padeada (PAD a cada lado): da
    // pendiente y copas correctas en las costuras sin recomputar fbm de más.
    let pw = dx as i32 + 2 * PAD;
    let pd = dz as i32 + 2 * PAD;
    let mut heights = vec![0u32; (pw * pd) as usize];
    for lz in 0..pd {
        for lx in 0..pw {
            let wx = ox + lx - PAD;
            let wz = oz + lz - PAD;
            heights[(lx + lz * pw) as usize] = column_height(wx, wz, dim, seed);
        }
    }
    // Altura en coordenada LOCAL de ventana (puede ser negativa hasta -PAD).
    let h_at = |lx: i32, lz: i32| heights[((lx + PAD) + (lz + PAD) * pw) as usize];

    // Terreno + agua, columna por columna de la ventana.
    for lz in 0..dz {
        for lx in 0..dx {
            let (li, lj) = (lx as i32, lz as i32);
            let (wx, wz) = (ox + li, oz + lj);
            let h = h_at(li, lj);
            // Pendiente: diferencia con vecinos → roca en acantilados.
            let slope = (h as i32 - h_at(li - 1, lj) as i32)
                .abs()
                .max((h as i32 - h_at(li, lj - 1) as i32).abs()) as f32;

            for y in 0..=h.min(dy - 1) {
                let fh = y as f32 / dy as f32;
                // Jitter por ruido en coordenadas de MUNDO (seamless entre ventanas).
                let jitter = hash2(wx, wz.wrapping_mul(31).wrapping_add(y as i32), seed ^ 0xABCD) * 0.06 - 0.03;
                let band = fh + jitter;
                let col = if y == h && slope > 2.5 && band > 0.34 {
                    rock
                } else if band < 0.33 {
                    sand
                } else if band < 0.55 {
                    mix(grass_lo, grass_hi, (band - 0.33) / 0.22)
                } else if band < 0.72 {
                    mix(grass_hi, rock, (band - 0.55) / 0.17)
                } else if band < 0.82 {
                    rock
                } else {
                    mix(rock, snow, (band - 0.82) / 0.10)
                };
                g.set(lx, y, lz, col);
            }

            // Agua: llena lo vacío bajo el nivel del mar (lagos/océano).
            if h < sea {
                for y in (h + 1)..=sea.min(dy - 1) {
                    let depth = (sea - y) as f32 / sea.max(1) as f32;
                    g.set(lx, y, lz, mix(shallow, deep, depth));
                }
            }
        }
    }

    // Árboles: roots barridos sobre la ventana padeada (±2) para que las copas
    // de árboles rooteados justo afuera asomen dentro, y las de borde se recorten.
    let leaf_base = [40, 96, 44];
    let leaf_hi = [62, 124, 58];
    let trunk = [96, 64, 38];
    for lz in -2..dz as i32 + 2 {
        for lx in -2..dx as i32 + 2 {
            let (wx, wz) = (ox + lx, oz + lz);
            let h = h_at(lx, lz);
            let fh = h as f32 / dy as f32;
            if h <= sea + 1 || fh < 0.33 || fh > 0.56 {
                continue;
            }
            if hash2(wx, wz, seed ^ 0x7717) > 0.016 {
                continue;
            }
            let th = 4 + (hash2(wx, wz, seed ^ 0x33) * 3.0) as u32;
            let top = (h + th).min(dy - 1);
            // Tronco (sólo si la columna cae dentro de la ventana).
            if (0..dx as i32).contains(&lx) && (0..dz as i32).contains(&lz) {
                for y in (h + 1)..=top {
                    g.set(lx as u32, y, lz as u32, trunk);
                }
            }
            // Copa: elipsoide de hojas, recortada a la ventana.
            let r = 2i32;
            let cy = top as i32;
            for dz2 in -r..=r {
                for dy2 in -r..=(r + 1) {
                    for dx2 in -r..=r {
                        if dx2 * dx2 + dy2 * dy2 + dz2 * dz2 > r * r + 1 {
                            continue;
                        }
                        let (gx, gy, gz) = (lx + dx2, cy + dy2, lz + dz2);
                        if (0..dx as i32).contains(&gx)
                            && (0..dy as i32).contains(&gy)
                            && (0..dz as i32).contains(&gz)
                        {
                            // Jitter de hoja por mundo (estable entre ventanas).
                            let v = hash2((wx + dx2) * 13 + gy, (wz + dz2) * 7 + gy, seed ^ 0x55) * 0.25;
                            g.set(gx as u32, gy as u32, gz as u32, mix(leaf_base, leaf_hi, v));
                        }
                    }
                }
            }
        }
    }
}

/// Genera un paisaje voxel en un grid `dim = [dx, dy, dz]` (con `y` arriba),
/// determinista por `seed`. El terreno ocupa hasta ~`0.85·dy` de alto; el nivel
/// del mar queda en `~0.30·dy`. Devuelve un [`VoxelGrid`] de `llimphi-3d` listo
/// para `VoxelRenderer`/`Scene3d`. Equivale a [`fill_terrain_window`] con
/// `origin = [0, 0]` (con el dirty reseteado: el grid es nuevo, el primer upload
/// es completo de todos modos).
pub fn terrain(dim: [u32; 3], seed: u32) -> VoxelGrid {
    let mut g = VoxelGrid::new(dim);
    fill_terrain_window(&mut g, [0, 0], seed);
    g.reset_dirty();
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `column_height` es función PURA de mundo: el mismo `(wx, wz)` da la misma
    /// altura aunque el origen de ventana cambie. Sin esto el streaming tendría
    /// costuras (escalones en los bordes de ventana).
    #[test]
    fn column_height_es_independiente_de_la_ventana() {
        let dim = [96, 48, 96];
        for &(wx, wz) in &[(0, 0), (37, -12), (-200, 5), (1000, -1000)] {
            let a = column_height(wx, wz, dim, 7);
            let b = column_height(wx, wz, dim, 7);
            assert_eq!(a, b, "determinista en ({wx},{wz})");
            assert!(a < dim[1], "altura dentro del mundo");
        }
    }

    /// Dos ventanas que solapan en mundo coinciden voxel-a-voxel en la zona
    /// común: una columna de mundo se ve igual desde cualquier ventana. Es la
    /// prueba dura de continuidad del streaming (sin GPU).
    #[test]
    fn ventanas_solapadas_coinciden_en_la_zona_comun() {
        let dim = [64, 40, 64];
        let seed = 4242;
        // Ventana A en origen (0,0); ventana B desplazada (+16,+16). Solapan en
        // el rango de mundo x,z ∈ [16, 64).
        let mut a = VoxelGrid::new(dim);
        fill_terrain_window(&mut a, [0, 0], seed);
        let mut b = VoxelGrid::new(dim);
        fill_terrain_window(&mut b, [16, 16], seed);

        let mut comparados = 0u32;
        for wz in 16..64u32 {
            for wx in 16..64u32 {
                for y in 0..dim[1] {
                    // mundo → local de cada ventana.
                    let va = a.get(wx, y, wz).unwrap();
                    let vb = b.get(wx - 16, y, wz - 16).unwrap();
                    assert_eq!(va, vb, "discrepan en mundo ({wx},{y},{wz})");
                    comparados += 1;
                }
            }
        }
        assert!(comparados > 10_000, "se compararon columnas de verdad");
    }
}
