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
