//! **Creador de mundos**: en vez de un terreno hardcodeado (como [`terrain`]
//! (crate::terrain), que es un mundo de pasto fijo), un [`WorldRecipe`]
//! *parametriza* el relieve y los materiales — y produce el [`VoxelGrid`]. Un
//! mundo concreto (desierto, pradera, …) es una **receta**, no una función nueva.
//!
//! Dos cosas que pidió el corto del desierto salen de acá:
//! - **Materiales** ([`Material`]): arena, agua, roca, cactus… cada uno con su
//!   color. El terreno se pinta por material, no por banda de altura cruda.
//! - **Receta del desierto** ([`WorldRecipe::desert`]): llano de arena, **pocas
//!   montañas**, **pocos ríos**, **cactus** ralos.
//!
//! La altura sigue siendo **función pura de mundo** ([`WorldRecipe::column_height`]):
//! mismo `(wx, wz)` → misma columna, así un mundo-receta también podrá streamear
//! (cuando se cablee a [`WorldStream`](crate::WorldStream)).

use llimphi_3d::VoxelGrid;
use serde::{Deserialize, Serialize};

use crate::terrain::{fbm, hash2, smooth, world_scale};

/// Un **material** del mundo: la unidad semántica con la que el creador pinta los
/// voxels (en vez de un color suelto). Da color y solidez; más adelante puede
/// llevar propiedades físicas (flotabilidad del agua, daño del cactus, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Material {
    /// Vacío (aire) — no se dibuja.
    Air,
    /// Arena del desierto.
    Sand,
    /// Tierra/pasto (praderas).
    Grass,
    /// Roca de acantilado / montaña.
    Rock,
    /// Nieve de cumbre.
    Snow,
    /// Agua (superficie de ríos/lagos/mar).
    Water,
    /// Carne de cactus (verde).
    Cactus,
}

impl Material {
    /// Color RGB del material.
    pub fn color(self) -> [u8; 3] {
        match self {
            Material::Air => [0, 0, 0],
            Material::Sand => [214, 188, 130],
            Material::Grass => [84, 140, 64],
            Material::Rock => [108, 102, 98],
            Material::Snow => [236, 240, 250],
            Material::Water => [54, 118, 158],
            Material::Cactus => [74, 128, 70],
        }
    }

    /// `true` salvo para [`Material::Air`].
    pub fn is_solid(self) -> bool {
        !matches!(self, Material::Air)
    }

    /// Todos los materiales, en orden de catálogo (para que un editor cicle entre
    /// ellos).
    pub const ALL: [Material; 7] = [
        Material::Air,
        Material::Sand,
        Material::Grass,
        Material::Rock,
        Material::Snow,
        Material::Water,
        Material::Cactus,
    ];

    /// Nombre legible (español) para la UI.
    pub fn label(self) -> &'static str {
        match self {
            Material::Air => "aire",
            Material::Sand => "arena",
            Material::Grass => "pasto",
            Material::Rock => "roca",
            Material::Snow => "nieve",
            Material::Water => "agua",
            Material::Cactus => "cactus",
        }
    }

    /// El material siguiente en el catálogo (cicla) — para botones de ciclo.
    pub fn next(self) -> Material {
        let i = Material::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Material::ALL[(i + 1) % Material::ALL.len()]
    }
}

/// Qué planta esparce un mundo y con qué forma.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Flora {
    /// Sin vegetación.
    None,
    /// Cactus columnar (tronco + algún brazo) — el desierto.
    Cactus,
}

impl Flora {
    /// Todas las opciones de flora (para ciclar en un editor).
    pub const ALL: [Flora; 2] = [Flora::None, Flora::Cactus];

    /// Nombre legible (español) para la UI.
    pub fn label(self) -> &'static str {
        match self {
            Flora::None => "ninguna",
            Flora::Cactus => "cactus",
        }
    }

    /// La flora siguiente (cicla).
    pub fn next(self) -> Flora {
        let i = Flora::ALL.iter().position(|&f| f == self).unwrap_or(0);
        Flora::ALL[(i + 1) % Flora::ALL.len()]
    }
}

/// La **receta de un mundo**: parámetros del relieve + materiales + flora. Producí
/// el `VoxelGrid` con [`generate`](Self::generate). Presets: [`desert`](Self::desert),
/// [`grassland`](Self::grassland).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WorldRecipe {
    /// Semilla del ruido (mismo seed → mismo mundo).
    pub seed: u32,
    /// Nivel base del suelo, fracción del alto del mundo `[0,1]` (la llanura).
    pub base: f32,
    /// Amplitud de las ondulaciones suaves del suelo (dunas), fracción del alto.
    pub dune: f32,
    /// Amplitud de las montañas, fracción del alto.
    pub relief: f32,
    /// **Densidad de montañas** `[0,1]`: 0 = casi todo llano (sólo pocos picos
    /// asoman), 1 = relieve por todos lados.
    pub mountains: f32,
    /// Nivel del agua, fracción del alto `[0,1]`: las depresiones por debajo se
    /// llenan de [`Material::Water`].
    pub water_level: f32,
    /// **Densidad de ríos** `[0,1]`: 0 = sin ríos; mayor = canales más anchos/
    /// frecuentes tallados hacia el agua.
    pub rivers: f32,
    /// Material de la superficie sólida (arena en el desierto, pasto en pradera).
    pub ground: Material,
    /// Material de acantilados/altura (roca).
    pub cliff: Material,
    /// Material de cumbre por encima de `peak_at` (nieve); `Air` = sin cumbre.
    pub peak: Material,
    /// Altura normalizada `[0,1]` a partir de la cual aparece `peak`.
    pub peak_at: f32,
    /// Flora y su densidad `[0,1]`.
    pub flora: Flora,
    pub flora_density: f32,
}

impl WorldRecipe {
    /// **Desierto llano**: arena, pocas montañas, pocos ríos, cactus ralos. El
    /// mundo de apertura del corto.
    pub fn desert(seed: u32) -> Self {
        Self {
            seed,
            base: 0.30,
            dune: 0.05,    // ondulación suave (dunas bajas)
            relief: 0.45,  // las pocas montañas que hay, altas
            mountains: 0.12, // pero MUY ralas
            water_level: 0.26, // por debajo del suelo base → casi sin agua salvo ríos
            rivers: 0.18,  // pocos ríos
            ground: Material::Sand,
            cliff: Material::Rock,
            peak: Material::Air, // sin nieve en el desierto
            peak_at: 1.0,
            flora: Flora::Cactus,
            flora_density: 0.010,
        }
    }

    /// **Pradera**: el mundo verde clásico (equivalente conceptual a [`terrain`]
    /// (crate::terrain)), expresado como receta para mostrar que el creador es
    /// general.
    pub fn grassland(seed: u32) -> Self {
        Self {
            seed,
            base: 0.22,
            dune: 0.10,
            relief: 0.7,
            mountains: 0.5,
            water_level: 0.30,
            rivers: 0.25,
            ground: Material::Grass,
            cliff: Material::Rock,
            peak: Material::Snow,
            peak_at: 0.80,
            flora: Flora::None,
            flora_density: 0.0,
        }
    }

    /// **Altura del terreno** (índice `y` del voxel sólido superior) en la columna
    /// de **mundo** `(wx, wz)`. Función pura: mismo punto → misma altura en
    /// cualquier ventana (continuidad para el streaming). Combina llanura base +
    /// dunas suaves + montañas *gated* (sólo asoman donde el fbm supera un umbral,
    /// así `mountains` bajo deja casi todo plano) + ríos tallados hacia el agua.
    pub fn column_height(&self, wx: i32, wz: i32, dim: [u32; 3]) -> u32 {
        let dy = dim[1] as f32;
        let scale = world_scale(dim);
        let s = self.seed;
        let (fx, fz) = (wx as f32 * scale, wz as f32 * scale);

        let base = self.base * dy;
        // Dunas: ondulación suave centrada en cero.
        let dunes = (fbm(fx * 1.7, fz * 1.7, 4, s ^ 0x11) - 0.5) * 2.0;
        let dune_h = dunes * self.dune * dy;
        // Montañas gated: sólo la parte alta del fbm sobresale. `mountains` baja el
        // umbral → más raras y aisladas.
        let c = fbm(fx, fz, 6, s);
        let thr = 1.0 - self.mountains.clamp(0.0, 1.0);
        let m = ((c - thr).max(0.0) / (1.0 - thr).max(1e-3)).clamp(0.0, 1.0);
        let mtn_h = smooth(m) * self.relief * dy;

        let mut h = base + dune_h + mtn_h;

        // Ríos: una "cresta" de ruido (ridged) marca líneas; cerca de la línea el
        // terreno se hunde hasta un lecho bajo el nivel del agua.
        if self.rivers > 0.0 {
            let r = 1.0 - (fbm(fx * 0.8, fz * 0.8, 4, s ^ 0x77) - 0.5).abs() * 2.0;
            let width = 0.03 + 0.10 * self.rivers.clamp(0.0, 1.0);
            if r > 1.0 - width {
                let t = ((r - (1.0 - width)) / width).clamp(0.0, 1.0);
                let bed = (self.water_level * dy - 2.0).max(1.0);
                h += (bed - h) * smooth(t);
            }
        }

        (h.clamp(1.0, dy - 1.0)) as u32
    }

    /// Nivel del agua como índice `y`.
    #[inline]
    pub fn water_y(&self, dim: [u32; 3]) -> u32 {
        (self.water_level * dim[1] as f32) as u32
    }

    /// Material del voxel sólido `(wx, y, wz)` de una columna de altura `h` con
    /// `slope` (desnivel con vecinos). Decide arena/roca/nieve por altura+pendiente.
    fn ground_material(&self, wx: i32, wz: i32, y: u32, h: u32, slope: f32, dim: [u32; 3]) -> Material {
        let dy = dim[1] as f32;
        let fh = y as f32 / dy;
        // Acantilado: la cara superior en pendiente fuerte es roca.
        if y == h && slope > 2.5 {
            return self.cliff;
        }
        // Cumbre nevada (si la receta la tiene).
        if self.peak.is_solid() && fh > self.peak_at {
            return self.peak;
        }
        // Jitter para que la transición a roca no sea una línea perfecta.
        let jitter = hash2(wx, wz.wrapping_mul(31).wrapping_add(y as i32), self.seed ^ 0xABCD) * 0.06 - 0.03;
        // Roca alta general (debajo de la cumbre): más arriba de 0.72 del alto.
        if fh + jitter > 0.72 {
            return self.cliff;
        }
        self.ground
    }

    /// `true` si en la columna `(wx, wz)` brota una planta (gate de densidad por
    /// hash de mundo → determinista y seamless). Sólo en suelo seco y llano.
    fn has_flora(&self, wx: i32, wz: i32, h: u32, slope: f32, dim: [u32; 3]) -> bool {
        if self.flora == Flora::None || self.flora_density <= 0.0 {
            return false;
        }
        if h <= self.water_y(dim) + 1 || slope > 1.5 {
            return false; // ni en el agua/orilla ni en pendiente
        }
        hash2(wx.wrapping_mul(7), wz.wrapping_mul(13), self.seed ^ 0xC4C7) < self.flora_density
    }

    /// **Construye el mundo**: rellena un `VoxelGrid` de tamaño `dim` con la esquina
    /// en la columna de mundo `origin = [wx, wz]` (usá `[0,0]` para el mundo
    /// centrado). Terreno + agua + flora, todo por material. Deja el grid limpio
    /// de *dirty* (se sube entero).
    pub fn generate_window(&self, dim: [u32; 3], origin: [i32; 2]) -> VoxelGrid {
        let [dx, dy, dz] = dim;
        let mut g = VoxelGrid::new(dim);
        let (ox, oz) = (origin[0], origin[1]);
        let water_y = self.water_y(dim);

        let h_at = |lx: i32, lz: i32| self.column_height(ox + lx, oz + lz, dim);

        for lz in 0..dz as i32 {
            for lx in 0..dx as i32 {
                let (wx, wz) = (ox + lx, oz + lz);
                let h = h_at(lx, lz);
                let slope = (h as i32 - h_at(lx - 1, lz) as i32)
                    .abs()
                    .max((h as i32 - h_at(lx, lz - 1) as i32).abs()) as f32;

                // Columna sólida.
                for y in 0..=h.min(dy - 1) {
                    let m = self.ground_material(wx, wz, y, h, slope, dim);
                    g.set(lx as u32, y, lz as u32, m.color());
                }
                // Agua: llena por encima del terreno hasta el nivel del agua.
                if h < water_y {
                    for y in (h + 1)..=water_y.min(dy - 1) {
                        g.set(lx as u32, y, lz as u32, Material::Water.color());
                    }
                }
                // Flora.
                if self.has_flora(wx, wz, h, slope, dim) {
                    self.place_flora(&mut g, lx as u32, h + 1, lz as u32, wx, wz, dim);
                }
            }
        }

        g.reset_dirty();
        g
    }

    /// Mundo centrado en el origen (`origin = [0,0]`).
    pub fn generate(&self, dim: [u32; 3]) -> VoxelGrid {
        self.generate_window(dim, [0, 0])
    }

    /// Coloca una planta con la base en `(x, base_y, z)` (local al grid). Hoy sólo
    /// el **cactus** columnar: tronco de 3–6 de alto + 0–2 brazos en L.
    fn place_flora(&self, g: &mut VoxelGrid, x: u32, base_y: u32, z: u32, wx: i32, wz: i32, dim: [u32; 3]) {
        if self.flora != Flora::Cactus {
            return;
        }
        let dy = dim[1];
        let col = Material::Cactus.color();
        // Altura del tronco: 3..6 determinista por la columna.
        let th = 3 + (hash2(wx, wz, self.seed ^ 0x1357) * 4.0) as u32;
        for k in 0..th {
            let y = base_y + k;
            if y >= dy {
                break;
            }
            g.set(x, y, z, col);
        }
        // Brazos: un par de salientes laterales a media altura (forma de candelabro).
        let arm_seed = hash2(wx.wrapping_add(1), wz, self.seed ^ 0x2468);
        if th >= 4 && arm_seed > 0.45 {
            let ay = base_y + th / 2;
            // Brazo a +x: un voxel al costado y dos hacia arriba.
            let arm = [(x.wrapping_add(1), ay, z), (x.wrapping_add(1), ay + 1, z)];
            for &(ax, ya, az) in &arm {
                if ax < dim[0] && ya < dy {
                    g.set(ax, ya, az, col);
                }
            }
            if arm_seed > 0.7 && x > 0 {
                // Brazo opuesto a -x.
                let arm2 = [(x - 1, ay + 1, z), (x - 1, ay + 2, z)];
                for &(ax, ya, az) in &arm2 {
                    if ya < dy {
                        g.set(ax, ya, az, col);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn altura_es_funcion_pura_de_mundo() {
        let r = WorldRecipe::desert(7);
        let dim = [96, 48, 96];
        // El mismo punto de mundo da la misma altura, lo pidas desde donde lo pidas.
        assert_eq!(r.column_height(1234, -567, dim), r.column_height(1234, -567, dim));
    }

    #[test]
    fn desierto_es_mas_llano_que_pradera() {
        let dim = [128, 56, 128];
        let var = |r: &WorldRecipe| {
            let mut hs = Vec::new();
            for z in (0..400).step_by(7) {
                for x in (0..400).step_by(7) {
                    hs.push(r.column_height(x, z, dim) as f32);
                }
            }
            let mean = hs.iter().sum::<f32>() / hs.len() as f32;
            hs.iter().map(|h| (h - mean).powi(2)).sum::<f32>() / hs.len() as f32
        };
        let desert = var(&WorldRecipe::desert(42));
        let grass = var(&WorldRecipe::grassland(42));
        assert!(desert < grass, "el desierto es más llano: var {desert:.1} vs {grass:.1}");
    }

    #[test]
    fn el_desierto_pinta_arena_agua_y_cactus() {
        let dim = [128, 48, 128];
        let r = WorldRecipe::desert(3);
        let g = r.generate(dim);
        let mut seen = [false; 3]; // [arena, agua, cactus]
        for z in 0..dim[2] {
            for x in 0..dim[0] {
                for y in 0..dim[1] {
                    if let Some(c) = g.get(x, y, z) {
                        if c[3] == 0 {
                            continue;
                        }
                        let rgb = [c[0], c[1], c[2]];
                        if rgb == Material::Sand.color() {
                            seen[0] = true;
                        } else if rgb == Material::Water.color() {
                            seen[1] = true;
                        } else if rgb == Material::Cactus.color() {
                            seen[2] = true;
                        }
                    }
                }
            }
        }
        assert!(seen[0], "hay arena");
        assert!(seen[1], "hay agua (ríos)");
        assert!(seen[2], "hay cactus");
    }

    #[test]
    fn mismo_seed_mismo_mundo() {
        let dim = [64, 40, 64];
        let a = WorldRecipe::desert(99).generate(dim);
        let b = WorldRecipe::desert(99).generate(dim);
        // Un muestreo basta: deterministas.
        for (x, y, z) in [(10, 12, 10), (30, 8, 44), (60, 20, 5)] {
            assert_eq!(a.get(x, y, z), b.get(x, y, z), "({x},{y},{z}) difiere");
        }
    }
}
