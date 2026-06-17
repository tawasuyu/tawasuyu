//! World-gen procedural (primera rebanada de M6 — `MOTOR-VOXEL.md` §7/§11.2).
//!
//! Genera un **paisaje voxel** dentro de un [`VoxelGrid`] acotado a partir de un
//! `seed`: un heightmap por ruido de valor fractal (fbm) sobre el plano `x·z`,
//! coloreado por bandas de altura y pendiente (agua, arena, pasto, roca, nieve),
//! con agua a nivel del mar y algunos árboles. Es **contenido reusable** por
//! cualquier app de la suite (no sólo dominium): el motor ya tenía sólo la escena
//! de prueba sintética ([`VoxelGrid::demo_scene`]); esto da un mundo "de verdad"
//! para showreel y para ejercitar el brick pool sparse a escala.
//!
//! Sin dependencias de ruido: el `value noise` es un hash entero + interpolación
//! bilineal, sumado en octavas (fbm). Determinista por `seed`.

use crate::voxel::VoxelGrid;

/// Hash entero → `f32` en `[0, 1)`. Mezcla estilo PCG/xxhash chico, determinista.
#[inline]
fn hash2(x: i32, y: i32, seed: u32) -> f32 {
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
fn smooth(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Ruido de valor bilineal en `(x, y)` continuos sobre la lattice entera.
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
fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
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

/// Genera un paisaje voxel en un grid `dim = [dx, dy, dz]` (con `y` arriba),
/// determinista por `seed`. El terreno ocupa hasta ~`0.85·dy` de alto; el nivel
/// del mar queda en `~0.30·dy`. Reusa el camino normal `set`, así que el grid
/// resultante se sube/streamea como cualquier otro.
pub fn terrain(dim: [u32; 3], seed: u32) -> VoxelGrid {
    let mut g = VoxelGrid::new(dim);
    let [dx, dy, dz] = dim;
    let sea = (dy as f32 * 0.30) as u32;

    // Frecuencia base: ~4 colinas grandes a lo ancho del mundo, más detalle fino.
    let scale = 4.0 / dx.max(dz) as f32;
    let min_h = (dy as f32 * 0.03) as u32;
    let amp = dy as f32 * 0.95;

    // Heightmap por columna. Se usa el rango vertical COMPLETO para que haya de
    // todo: el fbm crudo (~0.3..0.7) se estira para que el tercio bajo caiga al
    // mar (océanos/lagos) y el alto llegue a roca/nieve; encima se suma un
    // término "ridged" sólo en lo elevado, para picos afilados con nieve creíble.
    let mut heights = vec![0u32; (dx * dz) as usize];
    for z in 0..dz {
        for x in 0..dx {
            let c = fbm(x as f32 * scale, z as f32 * scale, 6, seed);
            let e0 = ((c - 0.35) * 2.0).clamp(0.0, 1.0);
            let e = e0 * e0 * (3.0 - 2.0 * e0); // smoothstep → mesetas + valles
            let ridge =
                1.0 - (fbm(x as f32 * scale * 2.3, z as f32 * scale * 2.3, 5, seed ^ 99) - 0.5).abs() * 2.0;
            let e = (e + e * e * ridge * 0.55).min(1.0);
            let h = (min_h + (e * amp) as u32).min(dy - 1);
            heights[(x + z * dx) as usize] = h;
        }
    }

    let rock = [88, 86, 92];
    let snow = [236, 240, 250];
    let grass_lo = [54, 110, 52];
    let grass_hi = [96, 150, 70];
    let sand = [196, 182, 130];
    let deep = [22, 52, 96];
    let shallow = [44, 110, 150];

    for z in 0..dz {
        for x in 0..dx {
            let h = heights[(x + z * dx) as usize];
            // Pendiente: diferencia con vecinos → roca en acantilados.
            let hx = heights[(x.saturating_sub(1).min(dx - 1) + z * dx) as usize];
            let hz = heights[(x + z.saturating_sub(1).min(dz - 1) * dx) as usize];
            let slope = (h as i32 - hx as i32).abs().max((h as i32 - hz as i32).abs()) as f32;

            for y in 0..=h {
                let fh = y as f32 / dy as f32;
                // Banda de material por altura, con un poco de jitter por ruido.
                let jitter = hash2(x as i32, (y * 31 + z) as i32, seed ^ 0xABCD) * 0.06 - 0.03;
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
                g.set(x, y, z, col);
            }

            // Agua: llena lo vacío bajo el nivel del mar (lagos/océano). Color por
            // profundidad para dar lectura de fondo.
            if h < sea {
                for y in (h + 1)..=sea {
                    let depth = (sea - y) as f32 / sea.max(1) as f32;
                    g.set(x, y, z, mix(shallow, deep, depth));
                }
            }
        }
    }

    // Árboles: en columnas de pasto sobre el mar, con baja probabilidad.
    for z in 2..dz.saturating_sub(2) {
        for x in 2..dx.saturating_sub(2) {
            let h = heights[(x + z * dx) as usize];
            let fh = h as f32 / dy as f32;
            if h <= sea + 1 || fh < 0.33 || fh > 0.56 {
                continue;
            }
            if hash2(x as i32, z as i32, seed ^ 0x7717) > 0.016 {
                continue;
            }
            let trunk = [96, 64, 38];
            let leaf = [40, 96, 44];
            let th = 4 + (hash2(x as i32, z as i32, seed ^ 0x33) * 3.0) as u32;
            let top = (h + th).min(dy - 1);
            for y in (h + 1)..=top {
                g.set(x, y, z, trunk);
            }
            // Copa: pequeño elipsoide de hojas.
            let r = 2i32;
            let cy = top as i32;
            for dz2 in -r..=r {
                for dy2 in -r..=(r + 1) {
                    for dx2 in -r..=r {
                        if dx2 * dx2 + dy2 * dy2 + dz2 * dz2 > r * r + 1 {
                            continue;
                        }
                        let (lx, ly, lz) = (x as i32 + dx2, cy + dy2, z as i32 + dz2);
                        if lx >= 0 && ly >= 0 && lz >= 0 {
                            let v = hash2(lx * 13 + ly, lz * 7 + ly, seed ^ 0x55) * 0.25;
                            g.set(
                                lx as u32,
                                ly as u32,
                                lz as u32,
                                mix(leaf, [62, 124, 58], v),
                            );
                        }
                    }
                }
            }
        }
    }

    // Estado inicial: el upload completo lo cubre, no es "mutación".
    g.reset_dirty();
    g
}
