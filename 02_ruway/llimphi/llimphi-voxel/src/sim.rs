//! `sim` — **simulación de la ley [`Fluir`](crate::LeyKind::Fluir)**: lo que hace
//! que el agua sea *líquida*. Un autómata celular sobre la ocupación de un
//! [`VoxelGrid`]: cada celda de agua **cae con gravedad**, **se esparce a vecinos
//! horizontales** y **cae por las cornisas** (cascadas). Conserva la masa (sólo
//! reubica celdas) y **converge** (no slosh perpetuo): la lateral sólo procede si el
//! agua puede descender o si hay *carga* (agua encima), así una torre se desploma en
//! charco y un manto de 1 de profundidad se queda quieto.
//!
//! Es contenido puro de CPU: no toca GPU. La app aplica los cambios al grid
//! (`set`/`clear`) y sube lo *dirty* con [`VoxelRenderer::sync`]
//! (llimphi_3d::VoxelRenderer::sync). El paso es determinista (sin azar): mismas
//! condiciones → misma evolución.

use crate::ecuacion::{FieldDef, FieldEngine, Program};
use llimphi_3d::VoxelGrid;

const AIR: u8 = 0;
const SOLID: u8 = 1;
const WATER: u8 = 2;

/// Un nuevo estado de una celda producido por [`WaterSim::step`]: `(pos, estado)`
/// con `estado` ∈ {[`AIR`], [`WATER`]}. La app lo traduce a `clear`/`set` en el grid.
pub type CellChange = ([u32; 3], u8);

/// Estado de aire de una celda (para que el caller distinga sin exponer constantes).
pub const CELL_AIR: u8 = AIR;
/// Estado de agua de una celda.
pub const CELL_WATER: u8 = WATER;

/// Autómata celular del agua sobre la ocupación de un grid.
pub struct WaterSim {
    dim: [u32; 3],
    /// Ocupación: 0 aire, 1 sólido (terreno), 2 agua.
    occ: Vec<u8>,
    /// Lock por celda dentro de un paso (evita mover dos veces / oscilar).
    moved: Vec<bool>,
    /// Índices tocados en el paso (para limpiar `moved` barato).
    touched: Vec<usize>,
    /// Fila de agua más alta (tope del escaneo; el agua nunca sube).
    top: u32,
    /// Alterna el orden de los vecinos cada paso (simetría).
    parity: u32,
    /// Parámetros de la ley Fluir. `gravedad` `[0,1]` = ritmo de caída (1 = cae cada
    /// paso; menor = más viscoso, cae a tirones). `horizontal` `[0,1]` = ritmo de
    /// esparcido lateral (0 = no se esparce → líquido espeso que se apila; 1 = se
    /// esparce cada paso → líquido fino que se nivela).
    gravedad: f32,
    horizontal: f32,
    /// Acumuladores deterministas que convierten los ritmos `[0,1]` en "actúa este
    /// paso sí/no" sin azar (no hay `Math.random`).
    g_acc: f32,
    h_acc: f32,
}

impl WaterSim {
    /// Clasifica un grid: las celdas cuyo color es exactamente `agua` son agua; el
    /// resto de las sólidas, terreno. El agua del worldgen se pinta con el color de
    /// paleta sin grano, así el match exacto la identifica.
    pub fn from_grid(grid: &VoxelGrid, agua: [u8; 3]) -> Self {
        Self::with_params(grid, agua, 1.0, 1.0)
    }

    /// Como [`from_grid`](Self::from_grid) pero con los parámetros de la ley Fluir
    /// (`gravedad`, `horizontal`) que dicta el material líquido.
    pub fn with_params(grid: &VoxelGrid, agua: [u8; 3], gravedad: f32, horizontal: f32) -> Self {
        let dim = grid.dim();
        let n = (dim[0] * dim[1] * dim[2]) as usize;
        let mut occ = vec![AIR; n];
        let mut top = 0u32;
        for z in 0..dim[2] {
            for y in 0..dim[1] {
                for x in 0..dim[0] {
                    if let Some(c) = grid.get(x, y, z) {
                        if c[3] > 0 {
                            let i = (x + y * dim[0] + z * dim[0] * dim[1]) as usize;
                            if [c[0], c[1], c[2]] == agua {
                                occ[i] = WATER;
                                top = top.max(y);
                            } else {
                                occ[i] = SOLID;
                            }
                        }
                    }
                }
            }
        }
        Self {
            dim,
            occ,
            moved: vec![false; n],
            touched: Vec::new(),
            top,
            parity: 0,
            gravedad: gravedad.clamp(0.05, 1.0),
            horizontal: horizontal.clamp(0.0, 1.0),
            g_acc: 0.0,
            h_acc: 0.0,
        }
    }

    /// Cantidad de celdas de agua (conservada por `step`).
    pub fn water_count(&self) -> usize {
        self.occ.iter().filter(|&&c| c == WATER).count()
    }

    #[inline]
    fn idx(&self, x: u32, y: u32, z: u32) -> usize {
        (x + y * self.dim[0] + z * self.dim[0] * self.dim[1]) as usize
    }

    /// Ocupación en `(x,y,z)` con bordes: fuera en X/Z o bajo el piso = sólido
    /// (muros + suelo); por encima del tope vertical = aire.
    #[inline]
    fn at(&self, x: i32, y: i32, z: i32) -> u8 {
        if x < 0 || z < 0 || x >= self.dim[0] as i32 || z >= self.dim[2] as i32 || y < 0 {
            return SOLID;
        }
        if y >= self.dim[1] as i32 {
            return AIR;
        }
        self.occ[self.idx(x as u32, y as u32, z as u32)]
    }

    #[inline]
    fn is_free(&self, x: i32, y: i32, z: i32) -> bool {
        // Libre = aire dentro del grid (no muro, no agua, no tope).
        x >= 0
            && z >= 0
            && y >= 0
            && x < self.dim[0] as i32
            && y < self.dim[1] as i32
            && z < self.dim[2] as i32
            && self.occ[self.idx(x as u32, y as u32, z as u32)] == AIR
            && !self.moved[self.idx(x as u32, y as u32, z as u32)]
    }

    /// Avanza un paso del autómata. Devuelve las celdas que cambiaron de estado
    /// (para subir sólo eso al grid). Lista vacía = el agua se asentó.
    pub fn step(&mut self) -> Vec<CellChange> {
        // Limpiar el lock del paso anterior.
        for &i in &self.touched {
            self.moved[i] = false;
        }
        self.touched.clear();
        let mut changes: Vec<CellChange> = Vec::new();

        // Ritmos → "actúa este paso" deterministas (acumulador, sin azar). Con
        // gravedad/horizontal = 1 actúan siempre (comportamiento base); más bajos,
        // a tirones → líquido más viscoso/espeso.
        self.g_acc += self.gravedad;
        let fall = self.g_acc >= 1.0;
        if fall {
            self.g_acc -= 1.0;
        }
        self.h_acc += self.horizontal;
        let spread = self.h_acc >= 1.0;
        if spread {
            self.h_acc -= 1.0;
        }

        // Orden de vecinos horizontales, rotado por paridad (simetría).
        let dirs0: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
        let rot = (self.parity % 4) as usize;
        let mut dirs = [(0, 0); 4];
        for k in 0..4 {
            dirs[k] = dirs0[(k + rot) % 4];
        }

        let top = self.top.min(self.dim[1] - 1);
        for z in 0..self.dim[2] as i32 {
            for y in 0..=top as i32 {
                for x in 0..self.dim[0] as i32 {
                    let i = self.idx(x as u32, y as u32, z as u32);
                    if self.occ[i] != WATER || self.moved[i] {
                        continue;
                    }
                    // 1) Caer: si abajo hay aire libre (gatea la gravedad).
                    if fall && self.is_free(x, y - 1, z) {
                        self.relocate(i, (x, y - 1, z), &mut changes);
                        continue;
                    }
                    // El componente horizontal (diagonal y lateral) gatea por `spread`.
                    if !spread {
                        continue;
                    }
                    // 2) Caer en diagonal (flujo / cornisa → cascada): vecino libre
                    //    cuya celda de abajo también está libre.
                    let mut done = false;
                    for (dx, dz) in dirs {
                        if self.is_free(x + dx, y, z + dz) && self.at(x + dx, y - 1, z + dz) == AIR {
                            self.relocate(i, (x + dx, y, z + dz), &mut changes);
                            done = true;
                            break;
                        }
                    }
                    if done {
                        continue;
                    }
                    // 3) Esparcir lateral SÓLO si hay carga (agua justo debajo): una
                    //    torre se desploma en charco; un manto de 1 se queda quieto.
                    if self.at(x, y - 1, z) == WATER {
                        for (dx, dz) in dirs {
                            if self.is_free(x + dx, y, z + dz) {
                                self.relocate(i, (x + dx, y, z + dz), &mut changes);
                                break;
                            }
                        }
                    }
                }
            }
        }
        self.parity = self.parity.wrapping_add(1);
        changes
    }

    /// Mueve el agua de `from` (índice) a `to` (coord), registrando los cambios y
    /// bloqueando el destino para el resto del paso.
    fn relocate(&mut self, from: usize, to: (i32, i32, i32), changes: &mut Vec<CellChange>) {
        let (tx, ty, tz) = (to.0 as u32, to.1 as u32, to.2 as u32);
        let ti = self.idx(tx, ty, tz);
        self.occ[from] = AIR;
        self.occ[ti] = WATER;
        self.moved[ti] = true;
        self.touched.push(ti);
        // Posición de origen.
        let fz = from as u32 / (self.dim[0] * self.dim[1]);
        let rem = from as u32 % (self.dim[0] * self.dim[1]);
        let fy = rem / self.dim[0];
        let fx = rem % self.dim[0];
        changes.push(([fx, fy, fz], AIR));
        changes.push(([tx, ty, tz], WATER));
    }
}

/// Simulación de la ley [`Crecer`](crate::LeyKind::Crecer): una planta (material con
/// rol objeto) **crece de abajo hacia arriba** a su velocidad. Toma las celdas del
/// objeto en el grid, las esconde, y las va revelando en orden de altura. La
/// `velocidad` escala cuántas celdas se revelan por paso (a `1.0`, la planta entera
/// brota en ~[`GROW_TICKS`] pasos sin importar su tamaño).
pub struct GrowthSim {
    /// Celdas de la planta `(pos, color)`, ordenadas por altura ascendente.
    cells: Vec<([u32; 3], [u8; 3])>,
    /// Cuántas reveladas hasta ahora.
    idx: usize,
    /// Acumulador de la tasa de revelado (determinista).
    acc: f32,
    /// Celdas reveladas por paso (derivada de la velocidad y el tamaño).
    rate: f32,
}

/// Pasos en los que una planta crece del todo a velocidad `1.0` (~2 s a 30 fps).
pub const GROW_TICKS: f32 = 60.0;

impl GrowthSim {
    /// Junta las celdas del color de la planta y las ordena por altura. `velocidad`
    /// viene de la ley Crecer del material.
    pub fn from_grid(grid: &VoxelGrid, planta: [u8; 3], velocidad: f32) -> Self {
        let dim = grid.dim();
        let mut cells = Vec::new();
        for z in 0..dim[2] {
            for y in 0..dim[1] {
                for x in 0..dim[0] {
                    if let Some(c) = grid.get(x, y, z) {
                        if c[3] > 0 && [c[0], c[1], c[2]] == planta {
                            cells.push(([x, y, z], planta));
                        }
                    }
                }
            }
        }
        // Orden por altura (y), luego x,z para determinismo.
        cells.sort_by_key(|(p, _)| (p[1], p[0], p[2]));
        let len = cells.len().max(1) as f32;
        let rate = (velocidad.max(0.0) * len / GROW_TICKS).max(0.05);
        Self { cells, idx: 0, acc: 0.0, rate }
    }

    /// Las celdas de la planta (para esconderlas al arrancar el crecimiento).
    pub fn cells(&self) -> &[([u32; 3], [u8; 3])] {
        &self.cells
    }

    /// `true` si la planta ya terminó de crecer.
    pub fn done(&self) -> bool {
        self.idx >= self.cells.len()
    }

    /// Revela el siguiente lote de celdas (según la velocidad). Devuelve las celdas a
    /// dibujar este paso `(pos, color)`. Vacío cuando ya creció del todo.
    pub fn step(&mut self) -> Vec<([u32; 3], [u8; 3])> {
        if self.done() {
            return Vec::new();
        }
        self.acc += self.rate;
        let n = self.acc.floor() as usize;
        self.acc -= n as f32;
        let mut out = Vec::new();
        for _ in 0..n {
            if self.idx < self.cells.len() {
                out.push(self.cells[self.idx]);
                self.idx += 1;
            }
        }
        out
    }
}

/// Simulación de una ley [`Ecuacion`](crate::LeyKind::Ecuacion) **sobre un material
/// real del mundo**: corre el sistema de ecuaciones de campo en las celdas del material
/// y las **recolorea** por el valor del campo visible — el material queda donde está,
/// "vivo" según su ecuación (a diferencia de [`WaterSim`], que mueve celdas).
///
/// Clasifica las celdas por **proximidad de color** al color base (± `tol` por canal),
/// para alcanzar también materiales con *grano* (el terreno varía su color; el agua no).
/// Materiales de color parecido pueden mezclarse — es una aproximación de preview.
pub struct EcuacionSim {
    engine: FieldEngine,
    /// Celdas del material (posición fija; se clasifican una vez).
    cells: Vec<[u32; 3]>,
    /// Color base del material (para modular el brillo).
    base: [u8; 3],
    /// Campo que tiñe el color.
    vis: usize,
    /// Rango del campo visible (para normalizar).
    fmin: f32,
    fmax: f32,
    /// Último color emitido por celda (para subir sólo lo que cambió).
    last: Vec<[u8; 3]>,
}

impl EcuacionSim {
    /// Clasifica las celdas cercanas a `material` (± `tol` por canal), arma el motor de
    /// campo sobre el grid y siembra una perturbación determinista por celda (para que
    /// la ecuación arranque con estructura). `vis` es el campo que se pinta.
    pub fn from_grid(
        grid: &VoxelGrid,
        material: [u8; 3],
        tol: i32,
        campos: Vec<FieldDef>,
        vis: usize,
    ) -> Self {
        let dim = grid.dim();
        let mut cells = Vec::new();
        for z in 0..dim[2] {
            for y in 0..dim[1] {
                for x in 0..dim[0] {
                    if let Some(c) = grid.get(x, y, z) {
                        if c[3] > 0 && cerca([c[0], c[1], c[2]], material, tol) {
                            cells.push([x, y, z]);
                        }
                    }
                }
            }
        }
        let vis = vis.min(campos.len().saturating_sub(1));
        let (fmin, fmax) = campos.get(vis).map(|d| (d.min, d.max)).unwrap_or((0.0, 1.0));
        let mut engine = FieldEngine::new(dim, campos);
        let nf = engine.fields().len();
        for &[x, y, z] in &cells {
            for f in 0..nf {
                let (mn, mx) = {
                    let d = &engine.fields()[f];
                    (d.min, d.max)
                };
                let h = hash01(x, y, z, f as u32);
                engine.set(f as u16, x, y, z, mn + h * 0.6 * (mx - mn));
            }
        }
        let last = vec![[0u8; 3]; cells.len()];
        Self { engine, cells, base: material, vis, fmin, fmax, last }
    }

    /// Cantidad de celdas del material clasificadas.
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// Avanza un paso del campo y devuelve las celdas cuyo **color cambió**
    /// `(pos, color)`, para subir sólo eso a la GPU. El color modula el brillo del
    /// color base por el valor normalizado del campo visible (preserva el tono).
    pub fn step(&mut self, program: &Program, params: &[f32]) -> Vec<([u32; 3], [u8; 3])> {
        self.engine.step(program, params, 1.0);
        let span = (self.fmax - self.fmin).max(1e-6);
        let mut out = Vec::new();
        for (i, &[x, y, z]) in self.cells.iter().enumerate() {
            let t = ((self.engine.get(self.vis as u16, x, y, z) - self.fmin) / span).clamp(0.0, 1.0);
            let k = 0.35 + 0.65 * t;
            let col = [
                (self.base[0] as f32 * k) as u8,
                (self.base[1] as f32 * k) as u8,
                (self.base[2] as f32 * k) as u8,
            ];
            if col != self.last[i] {
                self.last[i] = col;
                out.push(([x, y, z], col));
            }
        }
        out
    }
}

/// `true` si `c` está a ≤ `tol` de `base` en los tres canales.
fn cerca(c: [u8; 3], base: [u8; 3], tol: i32) -> bool {
    (c[0] as i32 - base[0] as i32).abs() <= tol
        && (c[1] as i32 - base[1] as i32).abs() <= tol
        && (c[2] as i32 - base[2] as i32).abs() <= tol
}

/// Hash determinista → `[0,1)` de una celda + índice de campo (sin azar; para sembrar).
fn hash01(x: u32, y: u32, z: u32, f: u32) -> f32 {
    let mut h = x
        .wrapping_mul(0x9E37_79B1)
        ^ y.wrapping_mul(0x85EB_CA77)
        ^ z.wrapping_mul(0xC2B2_AE3D)
        ^ f.wrapping_mul(0x27D4_EB2F);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0x00FF_FFFF) as f32 / (0x0100_0000 as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Material;

    /// Grid con piso sólido y un poco de agua arriba, en una columna.
    fn grid_con_columna(dim: [u32; 3], agua: [u8; 3], col_x: u32, col_z: u32, altura: u32) -> VoxelGrid {
        let mut g = VoxelGrid::new(dim);
        // Piso en y=0.
        for z in 0..dim[2] {
            for x in 0..dim[0] {
                g.set(x, 0, z, Material::Rock.color());
            }
        }
        // Columna de agua sobre el piso.
        for k in 0..altura {
            g.set(col_x, 1 + k, col_z, agua);
        }
        g.reset_dirty();
        g
    }

    #[test]
    fn ecuacion_sim_clasifica_y_recolorea() {
        use crate::ecuacion::{Assign, Expr, FieldDef, Program, Symbols};
        // Grid con un parche plano de un material (color exacto, como el agua).
        let dim = [12, 1, 12];
        let mut g = VoxelGrid::new(dim);
        let mat = [120, 160, 80];
        for z in 3..9 {
            for x in 3..9 {
                g.set(x, 0, z, mat);
            }
        }
        g.reset_dirty();
        let sym = Symbols { campos: vec!["t".into()], params: vec!["k".into()] };
        let e = Expr::parse("k * lap(t)", &sym).unwrap();
        let prog = Program::compile(&[Assign { campo: 0, expr: e }]);
        let mut sim = EcuacionSim::from_grid(&g, mat, 8, vec![FieldDef::new("t", 0.0, 0.0, 1.0)], 0);
        assert_eq!(sim.cell_count(), 36, "clasificó el parche 6×6");
        let mut changed = 0;
        for _ in 0..30 {
            changed += sim.step(&prog, &[0.2]).len();
        }
        assert!(changed > 0, "la ecuación recoloreó celdas del material");
    }

    #[test]
    fn el_agua_cae_hasta_el_piso() {
        let agua = Material::Water.color();
        let dim = [8, 16, 8];
        let mut g = VoxelGrid::new(dim);
        for z in 0..dim[2] {
            for x in 0..dim[0] {
                g.set(x, 0, z, Material::Rock.color());
            }
        }
        g.set(3, 10, 3, agua); // una gota flotando
        g.reset_dirty();
        let mut sim = WaterSim::from_grid(&g, agua);
        assert_eq!(sim.water_count(), 1);
        for _ in 0..30 {
            sim.step();
        }
        // La gota terminó descansando sobre el piso (y=1) — no flotando.
        assert_eq!(sim.water_count(), 1, "no se pierde ni duplica agua");
        assert_eq!(sim.at(3, 1, 3), CELL_WATER, "el agua quedó sobre el piso");
        assert_eq!(sim.at(3, 10, 3), CELL_AIR, "ya no flota");
    }

    #[test]
    fn la_columna_se_esparce_y_se_asienta() {
        let agua = Material::Water.color();
        let dim = [16, 20, 16];
        let g = grid_con_columna(dim, agua, 8, 8, 10); // torre de 10 de agua
        let mut sim = WaterSim::from_grid(&g, agua);
        let n0 = sim.water_count();

        let mut last_changes = 1;
        let mut steps = 0;
        while last_changes > 0 && steps < 2000 {
            last_changes = sim.step().len();
            steps += 1;
        }
        // Conserva masa y converge (deja de moverse) en tiempo finito.
        assert_eq!(sim.water_count(), n0, "masa conservada");
        assert!(steps < 2000, "se asentó (sin slosh perpetuo) en {steps} pasos");

        // Se esparció: el charco final es más ancho que la columna inicial (1 celda).
        let mut ancho = 0;
        for x in 0..dim[0] as i32 {
            if sim.at(x, 1, 8) == CELL_WATER {
                ancho += 1;
            }
        }
        assert!(ancho > 1, "el agua se esparció horizontalmente (ancho={ancho})");
    }

    #[test]
    fn horizontal_cero_no_se_esparce_espeso() {
        // Líquido espeso (horizontal=0): cae pero NO se esparce → se apila en columna.
        let agua = Material::Water.color();
        let dim = [16, 20, 16];
        let g = grid_con_columna(dim, agua, 8, 8, 10);
        let mut sim = WaterSim::with_params(&g, agua, 1.0, 0.0);
        for _ in 0..200 {
            sim.step();
        }
        let mut ancho = 0;
        for x in 0..dim[0] as i32 {
            if sim.at(x, 1, 8) == CELL_WATER {
                ancho += 1;
            }
        }
        assert_eq!(ancho, 1, "sin componente horizontal el líquido no se esparce");
    }

    #[test]
    fn la_planta_crece_de_abajo_hacia_arriba() {
        // Una columna de "planta" (color cactus) de 6 de alto sobre un piso.
        let planta = Material::Cactus.color();
        let dim = [8, 16, 8];
        let mut g = VoxelGrid::new(dim);
        for k in 0..6 {
            g.set(4, 1 + k, 4, planta);
        }
        g.reset_dirty();
        let mut sim = GrowthSim::from_grid(&g, planta, 1.0);
        assert_eq!(sim.cells().len(), 6);

        // Revelar paso a paso: cada celda nueva está a una altura >= la anterior.
        let mut revealed: Vec<u32> = Vec::new();
        let mut steps = 0;
        while !sim.done() && steps < 1000 {
            for (pos, _) in sim.step() {
                revealed.push(pos[1]);
            }
            steps += 1;
        }
        assert_eq!(revealed.len(), 6, "se revelaron todas las celdas");
        // Monótono creciente en altura (crece hacia arriba).
        assert!(revealed.windows(2).all(|w| w[0] <= w[1]), "crece de abajo hacia arriba: {revealed:?}");
        assert_eq!(revealed[0], 1, "empieza por la base");
    }

    #[test]
    fn cae_por_una_cornisa_cascada() {
        // Una repisa (escalón) con agua arriba: debe caer por el borde.
        let agua = Material::Water.color();
        let dim = [16, 16, 8];
        let mut g = VoxelGrid::new(dim);
        // Piso bajo en y=0 en todo; una repisa sólida en y=1..=5 sobre la mitad x<8.
        for z in 0..dim[2] {
            for x in 0..dim[0] {
                g.set(x, 0, z, Material::Rock.color());
            }
        }
        for z in 0..dim[2] {
            for x in 0..8 {
                for y in 1..=5 {
                    g.set(x, y, z, Material::Rock.color());
                }
            }
        }
        // Agua sobre la repisa, cerca del borde (x=7).
        for z in 0..dim[2] {
            g.set(7, 6, z, agua);
        }
        g.reset_dirty();
        let mut sim = WaterSim::from_grid(&g, agua);
        let n0 = sim.water_count();
        for _ in 0..60 {
            sim.step();
        }
        assert_eq!(sim.water_count(), n0, "masa conservada al caer");
        // Algo de agua llegó al piso bajo (y=1) del lado sin repisa (x>=8).
        let mut llego_abajo = false;
        for z in 0..dim[2] as i32 {
            for x in 8..dim[0] as i32 {
                if sim.at(x, 1, z) == CELL_WATER {
                    llego_abajo = true;
                }
            }
        }
        assert!(llego_abajo, "el agua cayó por la cornisa hasta el nivel bajo");
    }
}
