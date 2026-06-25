//! `dominium-voxel` — el puente entre la sim de dominium y el motor 3D
//! (`llimphi-3d`). Voxeliza un [`World`] (5 capas `f32` + agentes) en un
//! [`VoxelGrid`] que el ray-marcher de Llimphi dibuja barato (ver
//! `MOTOR-VOXEL.md` §3.2).
//!
//! **Regla que respeta la spec de dominium**: la sim (`core`/`physics`) NO se
//! toca y mantiene cero deps gráficas. Este crate es un *consumidor* — el
//! análogo voxel de `dominium-canvas-llimphi`. Lo único que importa del render
//! iso es su **semántica**, y se reusa literal:
//!
//! - **Color por celda**: [`dominium_render_plan::cell_color`] — el mismo
//!   blend de las 5 capas que pinta la maqueta 2.5D. El mundo voxel *es* el de
//!   dominium, no una recoloración paralela.
//! - **Altura de columna**: [`ZWeights::z_of`] — el mismo relieve compuesto
//!   que el render iso eleva en rombos. Acá esa altura se vuelve voxels.
//!
//! El mapeo es una función pura `grid → VoxelGrid`: cada celda `(cx, cy)` de
//! dominium → una **columna** de voxels en `(x=cx, z=cy)`, alta según el
//! relieve y coloreada por sus capas. Los agentes → [`Entity3d`] (cajas
//! analíticas instanciadas, baratas). Determinista: ningún RNG, ningún float
//! divergente.

use dominium_core::World;
use dominium_iso::ZWeights;
use dominium_render_plan::{cell_color, Palette};
use llimphi_3d::{Entity3d, VoxelGrid};

/// Tope de entidades que el motor dibuja por frame (`VoxelRenderer` lo capa en
/// 64). Si la población supera esto, [`lemming_entities`] submuestrea de forma
/// pareja y reporta cuántas quedaron afuera (sin cap silencioso).
pub const MAX_VOXEL_ENTITIES: usize = 64;

/// Cómo traducir el mundo lógico a voxels. Son los controles cosméticos del
/// puente — no tocan la simulación (igual que `PlanConfig` para el iso).
#[derive(Debug, Clone, Copy)]
pub struct VoxelConfig {
    /// Altura del grid voxel (dimensión `Y`). Las columnas se capan a
    /// `max_height - 1`, así una montaña desbocada nunca desborda el grid.
    pub max_height: u32,
    /// Pisos sólidos bajo **toda** celda (la "roca madre"). Da un lecho a los
    /// mares (celdas de poca materia) para que se lean como agua sobre un
    /// fondo, no como un agujero al vacío.
    pub base_floor: u32,
    /// Voxels de altura por unidad de relieve (`ZWeights::z_of`). El relieve
    /// típico de tierra ronda decenas; con `0.35` y `max_height = 48` las
    /// llanuras quedan a media altura y los picos asoman sin clipear.
    pub height_scale: f32,
    /// Paleta de capas — la misma que el render iso. El color de cada voxel
    /// sale de `cell_color(world, idx, &palette)`.
    pub palette: Palette,
}

impl Default for VoxelConfig {
    fn default() -> Self {
        Self {
            max_height: 48,
            base_floor: 2,
            height_scale: 0.35,
            palette: Palette::default(),
        }
    }
}

/// Altura (cantidad de voxels sólidos, contados desde `y = 0`) de la columna de
/// la celda `idx`. Es `base_floor + z_of · height_scale`, redondeado y clampeado
/// a `[1, max_height - 1]`. Función pura y determinista: la usan tanto el
/// rellenado del terreno como el posado de las entidades, así un agente siempre
/// queda **sobre** su columna, nunca dentro de la roca.
pub fn column_height(world: &World, zw: &ZWeights, cfg: &VoxelConfig, idx: usize) -> u32 {
    let z = zw.z_of(&world.grid, idx).max(0.0);
    let h = cfg.base_floor as f32 + z * cfg.height_scale;
    let hi = cfg.max_height.saturating_sub(1).max(1) as i64;
    (h.round() as i64).clamp(1, hi) as u32
}

/// Voxeliza el terreno de `world`: una columna por celda, altura por relieve,
/// color por capas. Devuelve un [`VoxelGrid`] de `dim = [ancho, max_height,
/// alto]` listo para `VoxelRenderer::new`. Los agentes NO van acá — son
/// entidades ([`lemming_entities`]), no terreno.
pub fn voxelize(world: &World, zw: &ZWeights, cfg: &VoxelConfig) -> VoxelGrid {
    let g = &world.grid;
    let (gw, gh) = (g.width as u32, g.height as u32);
    let mut grid = VoxelGrid::new([gw, cfg.max_height.max(2), gh]);

    for cy in 0..g.height {
        for cx in 0..g.width {
            let idx = g.idx(cx, cy);
            let top = column_height(world, zw, cfg, idx);
            let base = cell_color(world, idx, &cfg.palette);
            for y in 0..top {
                // Sombreado vertical: el lecho va más oscuro y aclara hacia la
                // cima. Da volumen a la columna sin iluminación extra y lee la
                // profundidad de un vistazo (≈ el `shade` de las caras del iso).
                let t = if top > 1 {
                    y as f32 / (top - 1) as f32
                } else {
                    1.0
                };
                let k = 0.55 + 0.45 * t;
                let rgb = [
                    to_u8(base[0] * k),
                    to_u8(base[1] * k),
                    to_u8(base[2] * k),
                ];
                grid.set(cx as u32, y, cy as u32, rgb);
            }
        }
    }
    // El upload completo de `VoxelRenderer::new` cubre este estado inicial —
    // no es "mutación", así que limpiamos el dirty (igual que `demo_scene`).
    grid.reset_dirty();
    grid
}

/// Convierte los lemmings vivos en entidades para el pase voxel. Cada agente es
/// una caja chica posada **sobre** su columna, coloreada por su acción (el byte
/// que define su "oficio" emergente) — así la manada se lee como roles, no como
/// puntos idénticos, fiel al objetivo antropológico de dominium.
///
/// Si hay más de [`MAX_VOXEL_ENTITIES`] vivos, submuestrea con paso parejo y
/// devuelve `(entidades, n_descartados)` — nunca capa en silencio.
pub fn lemming_entities(
    world: &World,
    zw: &ZWeights,
    cfg: &VoxelConfig,
) -> (Vec<Entity3d>, usize) {
    let lem = &world.lemmings;
    let n = lem.len();
    let g = &world.grid;
    let stride = n.div_ceil(MAX_VOXEL_ENTITIES).max(1);

    let mut out = Vec::with_capacity(MAX_VOXEL_ENTITIES.min(n));
    let mut i = 0;
    while i < n && out.len() < MAX_VOXEL_ENTITIES {
        let (px, py) = (lem.pos_x[i], lem.pos_y[i]);
        let (cx, cy) = g.clamp_cell(px, py);
        let top = column_height(world, zw, cfg, g.idx(cx, cy));
        out.push(Entity3d {
            // +0.5 centra el agente en la celda; la altura lo apoya sobre la
            // cima de la columna (top) más medio cuerpo.
            pos: [px + 0.5, top as f32 + 0.85, py + 0.5],
            half: [0.42, 0.85, 0.42],
            color: action_color(lem.accion[i]),
        });
        i += stride;
    }
    let shown = out.len();
    let dropped = n.saturating_sub(shown);
    (out, dropped)
}

/// Color por byte de acción (0..=5). Da identidad visual a cada "oficio"
/// emergente: el extractor saca color tierra, el que sincroniza va azul psique,
/// etc. Un byte fuera de rango cae a un gris neutro.
fn action_color(accion: u8) -> [u8; 3] {
    match accion {
        0 => [235, 235, 240], // Mover       — blanco (errante)
        1 => [205, 140, 70],  // Extraer     — terracota
        2 => [90, 150, 230],  // Sincronizar — azul psique
        3 => [90, 205, 130],  // Intercambiar— verde (reparte)
        4 => [240, 215, 90],  // Replicar    — oro (natalidad)
        5 => [220, 80, 75],   // Degradar    — rojo (conflicto)
        _ => [150, 150, 155],
    }
}

#[inline]
fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use dominium_core::World;

    fn cfg() -> VoxelConfig {
        VoxelConfig::default()
    }

    #[test]
    fn voxeliza_dentro_de_rango_y_no_vacio() {
        let mut w = World::new(8, 8);
        // Una loma de materia en el centro.
        let idx = w.grid.idx(4, 4);
        w.grid.materia[idx] = 60.0;
        let zw = ZWeights::default();
        let c = cfg();
        let g = voxelize(&w, &zw, &c);
        assert_eq!(g.dim(), [8, c.max_height, 8]);
        // El lecho (base_floor) siembra al menos esos voxels en cada columna.
        assert!(g.height_at(0, 0).is_some(), "toda columna tiene roca madre");
        // Ninguna columna desborda el grid.
        for z in 0..8 {
            for x in 0..8 {
                if let Some(top) = g.height_at(x, z) {
                    assert!(top < c.max_height, "columna ({x},{z}) clampeada");
                }
            }
        }
    }

    #[test]
    fn mas_materia_da_columna_mas_alta() {
        let mut w = World::new(4, 4);
        let baja = w.grid.idx(0, 0);
        let alta = w.grid.idx(1, 0);
        w.grid.materia[baja] = 10.0;
        w.grid.materia[alta] = 90.0;
        let zw = ZWeights::default(); // relieve = materia
        let c = cfg();
        let h_baja = column_height(&w, &zw, &c, baja);
        let h_alta = column_height(&w, &zw, &c, alta);
        assert!(h_alta > h_baja, "más materia ⇒ más alto: {h_alta} > {h_baja}");
    }

    #[test]
    fn mar_es_fino_y_tierra_es_gruesa() {
        // Con ZWeights default (relieve = materia), una celda de mar (mucha
        // psique, poca materia) queda casi al ras del lecho; una de tierra
        // (mucha materia) sube. Es lo que hace que el agua "se lea" plana.
        let mut w = World::new(4, 4);
        let mar = w.grid.idx(0, 0);
        let tierra = w.grid.idx(1, 0);
        w.grid.psique[mar] = 180.0;
        w.grid.materia[mar] = 1.0;
        w.grid.materia[tierra] = 70.0;
        let zw = ZWeights::default();
        let c = cfg();
        let h_mar = column_height(&w, &zw, &c, mar);
        let h_tierra = column_height(&w, &zw, &c, tierra);
        assert_eq!(h_mar, c.base_floor.max(1), "el mar queda al ras del lecho");
        assert!(h_tierra > h_mar, "la tierra sobresale del agua");
    }

    #[test]
    fn entidades_capadas_y_posadas_sobre_el_terreno() {
        let mut w = World::new(16, 16);
        // Una montaña para verificar que las entidades suben con el relieve.
        for cy in 0..16 {
            for cx in 0..16 {
                let idx = w.grid.idx(cx, cy);
                w.grid.materia[idx] = 40.0;
            }
        }
        // Más lemmings que el cap, para forzar el submuestreo.
        for k in 0..(MAX_VOXEL_ENTITIES * 3) {
            let x = (k % 16) as f32;
            let y = ((k / 16) % 16) as f32;
            w.lemmings.spawn(x, y, 50.0, [0.5; 4]);
        }
        let zw = ZWeights::default();
        let c = cfg();
        let (ents, dropped) = lemming_entities(&w, &zw, &c);
        assert!(ents.len() <= MAX_VOXEL_ENTITIES, "respeta el cap del motor");
        assert_eq!(
            ents.len() + dropped,
            w.lemmings.len(),
            "mostrados + descartados = población viva (sin pérdidas)"
        );
        assert!(dropped > 0, "con 3× el cap, algo se descarta (sin silencio)");
        // Toda entidad queda por encima del lecho (no enterrada).
        let h = column_height(&w, &zw, &c, w.grid.idx(0, 0));
        for e in &ents {
            assert!(e.pos[1] >= h as f32, "entidad sobre la columna, no dentro");
        }
    }
}
