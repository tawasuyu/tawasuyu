//! **Bioma**: el relieve + qué materiales lo pintan. Antes esto era `WorldRecipe`
//! con los materiales clavados en un `enum`; ahora un [`Bioma`] referencia
//! **materiales autorables por id** (los define el [`Project`](crate::Project)) y la
//! semilla vive en el [`Mundo`](crate::Mundo) que lo envuelve. El render no consume
//! ids: la app **resuelve** el bioma a una [`BiomaPalette`] (colores concretos) y la
//! pasa a [`Bioma::generate_window`].
//!
//! La altura sigue siendo **función pura de mundo** ([`Bioma::column_height`]):
//! mismo `(seed, wx, wz)` → misma columna, así un bioma también streamea
//! (continuidad de [`WorldStream`](crate::WorldStream)).
//!
//! El `enum` [`Material`] sobrevive **sólo como paleta semilla**: da los colores
//! de fábrica (arena, pasto, roca…) con los que el `Project` siembra sus
//! `MaterialDef` iniciales. No es el material autorable — ese es
//! [`MaterialDef`](crate::MaterialDef).

use llimphi_3d::VoxelGrid;
use serde::{Deserialize, Serialize};

use crate::terrain::{fbm, hash2, smooth, world_scale};

/// **Paleta semilla**: los materiales de fábrica con su color canónico. El
/// `Project` siembra un `MaterialDef` por variante (salvo `Air`) al arrancar, y la
/// UI puede ofrecerlos como punto de partida. No es el material autorable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// Color RGB de fábrica del material.
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

    /// **Grano de materia** de fábrica `[0,1]` (textura por vóxel; ver [`grained`]).
    pub fn grain(self) -> f32 {
        match self {
            Material::Sand => 0.55,
            Material::Grass | Material::Rock => 0.45,
            Material::Snow => 0.25,
            Material::Air | Material::Water | Material::Cactus => 0.0,
        }
    }

    /// `true` salvo para [`Material::Air`].
    pub fn is_solid(self) -> bool {
        !matches!(self, Material::Air)
    }

    /// Las variantes sembrables (sin `Air`), en orden de catálogo.
    pub const ALL: [Material; 6] = [
        Material::Sand,
        Material::Grass,
        Material::Rock,
        Material::Snow,
        Material::Water,
        Material::Cactus,
    ];

    /// Nombre legible (español).
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
}

/// **Forma de un objeto** colocable (un material con rol objeto). Hoy sólo el
/// cactus columnar; abierto a árboles, rocas sueltas, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Forma {
    /// Columna vertical con brazos (cactus).
    Columnar,
}

impl Forma {
    /// Todas las formas (para ciclar en un editor).
    pub const ALL: [Forma; 1] = [Forma::Columnar];

    /// Nombre legible (español).
    pub fn label(self) -> &'static str {
        match self {
            Forma::Columnar => "columnar",
        }
    }

    /// La forma siguiente (cicla).
    pub fn next(self) -> Forma {
        let i = Forma::ALL.iter().position(|&f| f == self).unwrap_or(0);
        Forma::ALL[(i + 1) % Forma::ALL.len()]
    }
}

/// **Material resuelto** para el render: color + grano concretos (ya aplicada la
/// herencia de [`MaterialDef`](crate::MaterialDef)). Es lo que entra a la paleta.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedMaterial {
    pub color: [u8; 3],
    pub grain: f32,
}

impl ResolvedMaterial {
    pub fn new(color: [u8; 3], grain: f32) -> Self {
        Self { color, grain }
    }
}

/// **Uso de un material como objeto** en un bioma: qué material, con qué densidad
/// `[0,1]` brota, y con qué [`Forma`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ObjetoUso {
    pub material: u64,
    pub densidad: f32,
    pub forma: Forma,
}

/// **Uso de un ser** en un bioma: qué ser y con qué probabilidad `[0,1]` puebla.
/// (Por ahora sólo dato del modelo; el spawn real vendrá con el motor.)
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SereUso {
    pub sere: u64,
    pub probabilidad: f32,
}

/// **Paleta resuelta de un bioma**: los colores concretos con los que pintar. La
/// app la arma resolviendo los ids del bioma contra los materiales del `Project`,
/// y se la pasa a [`Bioma::generate_window`]. Así worldgen no conoce ids ni herencia.
#[derive(Debug, Clone)]
pub struct BiomaPalette {
    pub ground: ResolvedMaterial,
    pub cliff: ResolvedMaterial,
    pub peak: Option<ResolvedMaterial>,
    /// Color del agua (superficie de ríos/lagos).
    pub agua: [u8; 3],
    /// Objetos a esparcir: `(material resuelto, densidad, forma)`.
    pub objetos: Vec<(ResolvedMaterial, f32, Forma)>,
}

impl BiomaPalette {
    /// Paleta de prueba/arranque rápida con un solo material de terreno (sin agua
    /// teñida especial ni objetos). Útil en tests y fallbacks.
    pub fn flat(color: [u8; 3], grain: f32) -> Self {
        let m = ResolvedMaterial::new(color, grain);
        Self {
            ground: m,
            cliff: ResolvedMaterial::new(Material::Rock.color(), Material::Rock.grain()),
            peak: None,
            agua: Material::Water.color(),
            objetos: Vec::new(),
        }
    }

    /// Material resuelto de la superficie de la columna en `(wx, wz, y)`: decide
    /// arena/roca/nieve por altura+pendiente (misma lógica que antes, pero sobre la
    /// paleta resuelta). `peak_at` viene del bioma.
    #[allow(clippy::too_many_arguments)]
    fn surface(
        &self,
        peak_at: f32,
        wx: i32,
        wz: i32,
        y: u32,
        h: u32,
        slope: f32,
        dim: [u32; 3],
        seed: u32,
    ) -> ResolvedMaterial {
        let dy = dim[1] as f32;
        let fh = y as f32 / dy;
        // Acantilado: la cara superior en pendiente fuerte es roca.
        if y == h && slope > 2.5 {
            return self.cliff;
        }
        // Cumbre (si la hay).
        if let Some(pk) = self.peak {
            if fh > peak_at {
                return pk;
            }
        }
        // Jitter para que la transición a roca no sea una línea perfecta.
        let jitter = hash2(wx, wz.wrapping_mul(31).wrapping_add(y as i32), seed ^ 0xABCD) * 0.06 - 0.03;
        if fh + jitter > 0.72 {
            return self.cliff;
        }
        self.ground
    }
}

/// Aplica el **grano de materia** al color base de un vóxel: perturba el brillo
/// (±`grain`·18%) y mete una pizca de variación por canal, deterministamente por la
/// posición de mundo. `grain = 0` → color intacto. Función pura → seamless entre
/// ventanas de streaming.
fn grained(base: [u8; 3], grain: f32, wx: i32, y: u32, wz: i32, seed: u32) -> [u8; 3] {
    if grain <= 0.0 {
        return base;
    }
    let g = grain.clamp(0.0, 1.0);
    let h = hash2(wx, wz.wrapping_mul(31).wrapping_add(y as i32 * 7), seed ^ 0x6817);
    let bright = 1.0 + (h - 0.5) * 2.0 * g * 0.18; // ±18% a grano pleno
    let speck = hash2(wx.wrapping_mul(13).wrapping_add(y as i32), wz, seed ^ 0x51A3);
    let tilt = (speck - 0.5) * 2.0 * g * 12.0; // ±12 niveles, rompe el liso plano
    let ch = |c: u8, extra: f32| (c as f32 * bright + extra).clamp(0.0, 255.0) as u8;
    [ch(base[0], tilt), ch(base[1], tilt * 0.7), ch(base[2], -tilt * 0.5)]
}

/// La **receta de relieve de un bioma**: parámetros del terreno + referencias a los
/// materiales (por id) que lo pintan. La **semilla NO vive acá** — la aporta el
/// [`Mundo`](crate::Mundo) al generar (un mismo bioma con semillas distintas da
/// mundos distintos). Producí el `VoxelGrid` con [`generate_window`](Self::generate_window),
/// pasándole la [`BiomaPalette`] resuelta.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bioma {
    /// Id estable (asignado por el [`Project`](crate::Project)).
    pub id: u64,
    /// Nombre editable.
    pub name: String,
    /// Nivel base del suelo, fracción del alto del mundo `[0,1]` (la llanura).
    pub base: f32,
    /// Amplitud de las ondulaciones suaves del suelo (dunas), fracción del alto.
    pub dune: f32,
    /// Amplitud de las montañas, fracción del alto.
    pub relief: f32,
    /// **Densidad de montañas** `[0,1]`: 0 = casi todo llano, 1 = relieve por todos lados.
    pub mountains: f32,
    /// Nivel del agua, fracción del alto `[0,1]`.
    pub water_level: f32,
    /// **Densidad de ríos** `[0,1]`.
    pub rivers: f32,
    /// Altura normalizada `[0,1]` a partir de la cual aparece la cumbre (`peak`).
    pub peak_at: f32,
    /// Material del suelo (id en [`Project::materiales`](crate::Project)).
    pub ground: u64,
    /// Material de acantilados/altura (id).
    pub cliff: u64,
    /// Material de cumbre por encima de `peak_at` (id); `None` = sin cumbre.
    pub peak: Option<u64>,
    /// Objetos a esparcir (flora/props), cada uno con su densidad y forma.
    #[serde(default)]
    pub objetos: Vec<ObjetoUso>,
    /// Seres que pueblan el bioma (sólo dato por ahora).
    #[serde(default)]
    pub seres: Vec<SereUso>,
}

impl Bioma {
    /// **Altura del terreno** (índice `y` del voxel sólido superior) en la columna
    /// de **mundo** `(wx, wz)` para la semilla `seed`. Función pura: mismo punto →
    /// misma altura en cualquier ventana. Llanura base + dunas + montañas *gated* +
    /// ríos tallados hacia el agua.
    pub fn column_height(&self, seed: u32, wx: i32, wz: i32, dim: [u32; 3]) -> u32 {
        let dy = dim[1] as f32;
        let scale = world_scale(dim);
        let (fx, fz) = (wx as f32 * scale, wz as f32 * scale);

        let base = self.base * dy;
        let dunes = (fbm(fx * 1.7, fz * 1.7, 4, seed ^ 0x11) - 0.5) * 2.0;
        let dune_h = dunes * self.dune * dy;
        let c = fbm(fx, fz, 6, seed);
        let thr = 1.0 - self.mountains.clamp(0.0, 1.0);
        let m = ((c - thr).max(0.0) / (1.0 - thr).max(1e-3)).clamp(0.0, 1.0);
        let mtn_h = smooth(m) * self.relief * dy;

        let mut h = base + dune_h + mtn_h;

        if self.rivers > 0.0 {
            let r = 1.0 - (fbm(fx * 0.8, fz * 0.8, 4, seed ^ 0x77) - 0.5).abs() * 2.0;
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

    /// `true` si en la columna `(wx, wz)` brota un objeto de densidad `densidad`
    /// (gate por hash de mundo, sembrado por `salt` para que objetos distintos no se
    /// pisen). Sólo en suelo seco y llano.
    fn has_objeto(&self, wx: i32, wz: i32, h: u32, slope: f32, densidad: f32, salt: u32, dim: [u32; 3]) -> bool {
        if densidad <= 0.0 {
            return false;
        }
        if h <= self.water_y(dim) + 1 || slope > 1.5 {
            return false;
        }
        hash2(wx.wrapping_mul(7), wz.wrapping_mul(13), self.seed_salt(salt)) < densidad
    }

    /// Mezcla un `salt` con una constante estable para sembrar el gate de objetos.
    #[inline]
    fn seed_salt(&self, salt: u32) -> u32 {
        (0xC4C7u32).wrapping_add(salt.wrapping_mul(0x9E37_79B9))
    }

    /// **Construye una ventana del bioma** con la esquina en la columna de mundo
    /// `origin = [wx, wz]`, semilla `seed` y la [`BiomaPalette`] ya resuelta. Terreno
    /// + agua + objetos, todo por material. Deja el grid limpio de *dirty*.
    pub fn generate_window(
        &self,
        seed: u32,
        palette: &BiomaPalette,
        dim: [u32; 3],
        origin: [i32; 2],
    ) -> VoxelGrid {
        let [dx, dy, dz] = dim;
        let mut g = VoxelGrid::new(dim);
        let (ox, oz) = (origin[0], origin[1]);
        let water_y = self.water_y(dim);

        let h_at = |lx: i32, lz: i32| self.column_height(seed, ox + lx, oz + lz, dim);

        for lz in 0..dz as i32 {
            for lx in 0..dx as i32 {
                let (wx, wz) = (ox + lx, oz + lz);
                let h = h_at(lx, lz);
                let slope = (h as i32 - h_at(lx - 1, lz) as i32)
                    .abs()
                    .max((h as i32 - h_at(lx, lz - 1) as i32).abs()) as f32;

                // Columna sólida (con grano por material).
                for y in 0..=h.min(dy - 1) {
                    let m = palette.surface(self.peak_at, wx, wz, y, h, slope, dim, seed);
                    g.set(lx as u32, y, lz as u32, grained(m.color, m.grain, wx, y, wz, seed));
                }
                // Agua.
                if h < water_y {
                    for y in (h + 1)..=water_y.min(dy - 1) {
                        g.set(lx as u32, y, lz as u32, palette.agua);
                    }
                }
                // Objetos (flora/props): cada uso, con su densidad y forma.
                for (i, (mat, densidad, forma)) in palette.objetos.iter().enumerate() {
                    if self.has_objeto(wx, wz, h, slope, *densidad, i as u32, dim) {
                        self.place_objeto(&mut g, *forma, mat.color, lx as u32, h + 1, lz as u32, wx, wz, dim);
                        break; // un objeto por columna
                    }
                }
            }
        }

        g.reset_dirty();
        g
    }

    /// Coloca un objeto con la base en `(x, base_y, z)` (local). Hoy sólo
    /// [`Forma::Columnar`]: tronco de 3–6 de alto + 0–2 brazos en L (cactus).
    #[allow(clippy::too_many_arguments)]
    fn place_objeto(
        &self,
        g: &mut VoxelGrid,
        forma: Forma,
        col: [u8; 3],
        x: u32,
        base_y: u32,
        z: u32,
        wx: i32,
        wz: i32,
        dim: [u32; 3],
    ) {
        match forma {
            Forma::Columnar => {
                let dy = dim[1];
                let th = 3 + (hash2(wx, wz, 0x1357) * 4.0) as u32;
                for k in 0..th {
                    let y = base_y + k;
                    if y >= dy {
                        break;
                    }
                    g.set(x, y, z, col);
                }
                let arm_seed = hash2(wx.wrapping_add(1), wz, 0x2468);
                if th >= 4 && arm_seed > 0.45 {
                    let ay = base_y + th / 2;
                    let arm = [(x.wrapping_add(1), ay, z), (x.wrapping_add(1), ay + 1, z)];
                    for &(ax, ya, az) in &arm {
                        if ax < dim[0] && ya < dy {
                            g.set(ax, ya, az, col);
                        }
                    }
                    if arm_seed > 0.7 && x > 0 {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un bioma de desierto de prueba (ids ficticios; la generación no los usa, usa
    /// la paleta) + su paleta de arena/roca.
    fn desierto() -> (Bioma, BiomaPalette) {
        let b = Bioma {
            id: 1,
            name: "desierto".into(),
            base: 0.30,
            dune: 0.05,
            relief: 0.45,
            mountains: 0.12,
            water_level: 0.26,
            rivers: 0.18,
            peak_at: 1.0,
            ground: 1,
            cliff: 2,
            peak: None,
            objetos: vec![ObjetoUso { material: 3, densidad: 0.010, forma: Forma::Columnar }],
            seres: vec![],
        };
        let pal = BiomaPalette {
            ground: ResolvedMaterial::new(Material::Sand.color(), Material::Sand.grain()),
            cliff: ResolvedMaterial::new(Material::Rock.color(), Material::Rock.grain()),
            peak: None,
            agua: Material::Water.color(),
            objetos: vec![(ResolvedMaterial::new(Material::Cactus.color(), 0.0), 0.010, Forma::Columnar)],
        };
        (b, pal)
    }

    fn pradera() -> (Bioma, BiomaPalette) {
        let b = Bioma {
            id: 2,
            name: "pradera".into(),
            base: 0.22,
            dune: 0.10,
            relief: 0.7,
            mountains: 0.5,
            water_level: 0.30,
            rivers: 0.25,
            peak_at: 0.80,
            ground: 4,
            cliff: 2,
            peak: Some(5),
            objetos: vec![],
            seres: vec![],
        };
        let pal = BiomaPalette {
            ground: ResolvedMaterial::new(Material::Grass.color(), Material::Grass.grain()),
            cliff: ResolvedMaterial::new(Material::Rock.color(), Material::Rock.grain()),
            peak: Some(ResolvedMaterial::new(Material::Snow.color(), Material::Snow.grain())),
            agua: Material::Water.color(),
            objetos: vec![],
        };
        (b, pal)
    }

    #[test]
    fn altura_es_funcion_pura_de_mundo() {
        let (b, _) = desierto();
        let dim = [96, 48, 96];
        assert_eq!(b.column_height(7, 1234, -567, dim), b.column_height(7, 1234, -567, dim));
    }

    #[test]
    fn desierto_es_mas_llano_que_pradera() {
        let dim = [128, 56, 128];
        let var = |b: &Bioma| {
            let mut hs = Vec::new();
            for z in (0..400).step_by(7) {
                for x in (0..400).step_by(7) {
                    hs.push(b.column_height(42, x, z, dim) as f32);
                }
            }
            let mean = hs.iter().sum::<f32>() / hs.len() as f32;
            hs.iter().map(|h| (h - mean).powi(2)).sum::<f32>() / hs.len() as f32
        };
        let d = var(&desierto().0);
        let g = var(&pradera().0);
        assert!(d < g, "el desierto es más llano: var {d:.1} vs {g:.1}");
    }

    #[test]
    fn el_desierto_pinta_arena_agua_y_cactus() {
        let dim = [128, 48, 128];
        let (b, pal) = desierto();
        let grid = b.generate_window(3, &pal, dim, [0, 0]);
        let mut seen = [false; 3]; // [arena, agua, cactus]
        let near = |a: [u8; 3], b: [u8; 3]| {
            (a[0] as i32 - b[0] as i32).abs() <= 60
                && (a[1] as i32 - b[1] as i32).abs() <= 60
                && (a[2] as i32 - b[2] as i32).abs() <= 60
        };
        for z in 0..dim[2] {
            for x in 0..dim[0] {
                for y in 0..dim[1] {
                    if let Some(c) = grid.get(x, y, z) {
                        if c[3] == 0 {
                            continue;
                        }
                        let rgb = [c[0], c[1], c[2]];
                        if rgb == Material::Water.color() {
                            seen[1] = true;
                        } else if rgb == Material::Cactus.color() {
                            seen[2] = true;
                        } else if near(rgb, Material::Sand.color()) {
                            seen[0] = true;
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
    fn grano_de_materia_perturba_pero_es_determinista() {
        let base = Material::Sand.color();
        assert_eq!(grained(base, 0.0, 5, 3, 9, 1337), base);
        let g1 = grained(base, 0.6, 5, 3, 9, 1337);
        assert_ne!(g1, base);
        let g2 = grained(base, 0.6, 5, 3, 9, 1337);
        assert_eq!(g1, g2);
        let gn = grained(base, 0.6, 6, 3, 9, 1337);
        assert_ne!(g1, gn);
        assert!(Material::Sand.grain() > 0.0);
    }

    #[test]
    fn mismo_seed_mismo_mundo() {
        let dim = [64, 40, 64];
        let (b, pal) = desierto();
        let a = b.generate_window(99, &pal, dim, [0, 0]);
        let c = b.generate_window(99, &pal, dim, [0, 0]);
        for (x, y, z) in [(10, 12, 10), (30, 8, 44), (60, 20, 5)] {
            assert_eq!(a.get(x, y, z), c.get(x, y, z), "({x},{y},{z}) difiere");
        }
    }
}
