//! `dominium-render-plan` — la maqueta isométrica, agnóstica de backend.
//!
//! El último eslabón antes de la pantalla. Toma un [`World`] lógico, lo
//! proyecta con un [`IsoProjector`] y emite una lista plana de
//! [`Quad`]s 2D ya ordenados de atrás hacia adelante: cualquier backend
//! (GPUI, `<canvas>` web, TUI) sólo tiene que pintarlos en orden.
//!
//! Aquí no hay `gpui`, ni `wgpu`, ni `f64`: sólo aritmética `f32` y
//! `dominium-iso`. La regla de la spec —cero dependencias gráficas en el
//! núcleo— se respeta hasta el penúltimo crate.
//!
//! ```text
//!   World ──► build_plan(iso, weights, cfg) ──► RenderPlan { quads }
//!                                                    │
//!                          backend.paint(quad) ◄─────┘  (en orden)
//! ```
//!
//! - Una celda → un quad-rombo aproximado, coloreado por la mezcla de sus
//!   5 capas (la altura sale del `Z` compuesto, el color de la psique del
//!   suelo).
//! - Un Lemming → un quad-marca posado sobre el relieve de su celda.
//! - Todo se ordena por `depth = x + y` (orden de pintor isométrico).

#![forbid(unsafe_code)]

use dominium_core::World;
use dominium_iso::{IsoProjector, ZWeights};
use serde::{Deserialize, Serialize};

/// Color RGBA lineal, componentes en `0.0..=1.0`.
pub type Color = [f32; 4];

/// Un rectángulo 2D en coordenadas de pantalla, listo para pintar. El
/// origen `(0,0)` es el centro de la proyección; el backend traslada.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quad {
    /// Esquina superior-izquierda, eje X de pantalla.
    pub x: f32,
    /// Esquina superior-izquierda, eje Y de pantalla.
    pub y: f32,
    /// Ancho en pixels.
    pub w: f32,
    /// Alto en pixels.
    pub h: f32,
    /// Color RGBA.
    pub color: Color,
    /// Clave de orden de pintor: menor = más al fondo. El plan ya viene
    /// ordenado, pero se conserva por si el backend reordena.
    pub depth: f32,
}

/// Paleta: un color por capa de la grilla, más el de los Lemmings. El
/// color de cada celda es la mezcla de estos pesada por el valor relativo
/// de cada capa.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Palette {
    /// Color de una celda sin ningún campo (terreno desnudo).
    pub floor: Color,
    pub materia: Color,
    pub psique: Color,
    pub poder: Color,
    pub oro: Color,
    pub degradacion: Color,
    /// Color de la marca de un Lemming.
    pub lemming: Color,
    /// Color del aura de influencia de un Concepto (translúcida).
    pub concepto_aura: Color,
    /// Color de la marca central de un Concepto.
    pub concepto: Color,
}

impl Default for Palette {
    /// Paleta "tablero psicológico": verde materia, azul psique, rojo
    /// poder, ámbar oro, violeta degradación.
    fn default() -> Self {
        Self {
            floor: [0.10, 0.11, 0.13, 1.0],
            materia: [0.30, 0.72, 0.38, 1.0],
            psique: [0.32, 0.55, 0.86, 1.0],
            poder: [0.84, 0.27, 0.24, 1.0],
            oro: [0.90, 0.74, 0.24, 1.0],
            degradacion: [0.52, 0.30, 0.62, 1.0],
            lemming: [0.96, 0.96, 0.98, 1.0],
            concepto_aura: [0.95, 0.86, 0.55, 0.18],
            concepto: [0.98, 0.88, 0.42, 1.0],
        }
    }
}

/// Ajustes de la maqueta: tamaños de quad y paleta. Lo que un panel
/// expondría como controles de presentación (no afectan la simulación).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PlanConfig {
    /// Lado del quad de una celda, en pixels.
    pub tile: f32,
    /// Lado del quad-marca de un Lemming, en pixels.
    pub lemming_size: f32,
    /// Cuánto se eleva la marca del Lemming sobre el relieve de su celda,
    /// en unidades de `Z`.
    pub lemming_lift: f32,
    /// Lado del quad-marca central de un Concepto, en pixels.
    pub concepto_size: f32,
    /// Cuánto se eleva la marca de un Concepto sobre el relieve, en `Z`.
    pub concepto_lift: f32,
    pub palette: Palette,
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self {
            tile: 18.0,
            lemming_size: 9.0,
            lemming_lift: 0.6,
            concepto_size: 14.0,
            concepto_lift: 1.4,
            palette: Palette::default(),
        }
    }
}

/// Lista de quads ordenada de atrás hacia adelante + caja envolvente.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RenderPlan {
    /// Quads ya ordenados por `depth` ascendente: píntalos en orden.
    pub quads: Vec<Quad>,
    /// Caja envolvente de todos los quads — el backend la usa para
    /// centrar o escalar la vista.
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl RenderPlan {
    /// Ancho de la caja envolvente.
    pub fn width(&self) -> f32 {
        self.max_x - self.min_x
    }

    /// Alto de la caja envolvente.
    pub fn height(&self) -> f32 {
        self.max_y - self.min_y
    }
}

/// Mezcla `n` colores con pesos: `Σ wᵢ·colorᵢ / Σ wᵢ`. Alpha del primero.
fn blend(parts: &[(f32, Color)]) -> Color {
    let total: f32 = parts.iter().map(|(w, _)| *w).sum();
    if total <= f32::EPSILON {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let mut out = [0.0f32; 4];
    for (w, c) in parts {
        let k = w / total;
        for ch in 0..3 {
            out[ch] += k * c[ch];
        }
    }
    out[3] = 1.0;
    out
}

/// Color de una celda: mezcla de la paleta pesada por el valor relativo
/// de sus 5 capas. Una celda vacía cae al color `floor`.
fn cell_color(world: &World, idx: usize, pal: &Palette) -> Color {
    let g = &world.grid;
    let layers = [
        (g.materia[idx].max(0.0), pal.materia),
        (g.psique[idx].max(0.0), pal.psique),
        (g.poder[idx].max(0.0), pal.poder),
        (g.oro[idx].max(0.0), pal.oro),
        (g.degradacion[idx].max(0.0), pal.degradacion),
    ];
    let total: f32 = layers.iter().map(|(v, _)| *v).sum();
    if total <= f32::EPSILON {
        return pal.floor;
    }
    blend(&layers)
}

/// Construye la maqueta isométrica de un `World`.
///
/// Emite un quad por celda (coloreado por sus capas, elevado por el `Z`
/// compuesto de `weights`) y un quad-marca por Lemming vivo, posado sobre
/// el relieve de su celda. El resultado viene ordenado por profundidad de
/// pintor: el backend sólo recorre `plan.quads` y pinta.
pub fn build_plan(
    world: &World,
    iso: &IsoProjector,
    weights: &ZWeights,
    cfg: &PlanConfig,
) -> RenderPlan {
    let g = &world.grid;
    let mut quads: Vec<Quad> = Vec::with_capacity(g.cells() + world.lemmings.len());

    // --- Celdas: un quad-rombo aproximado por celda ---
    for cy in 0..g.height {
        for cx in 0..g.width {
            let idx = g.idx(cx, cy);
            let z = weights.z_of(g, idx);
            let (sx, sy) = iso.project(cx as f32, cy as f32, z);
            quads.push(Quad {
                x: sx - cfg.tile * 0.5,
                y: sy - cfg.tile * 0.5,
                w: cfg.tile,
                h: cfg.tile,
                color: cell_color(world, idx, &cfg.palette),
                depth: cx as f32 + cy as f32,
            });
        }
    }

    // --- Conceptos: aura translúcida + marca central ---
    // El aura va al nivel del suelo (z=0), antes de las celdas con la misma
    // diagonal: pinta como un halo bajo lemmings y celdas. La marca central
    // va por encima de los lemmings (+0.75) — un Concepto es una entidad
    // saliente, no enterrada en el campo.
    for c in &world.conceptos.items {
        let (sx, sy) = iso.project(c.pos_x, c.pos_y, 0.0);
        let aura = c.radius * 2.0 * cfg.tile;
        quads.push(Quad {
            x: sx - aura * 0.5,
            y: sy - aura * 0.5,
            w: aura,
            h: aura,
            color: cfg.palette.concepto_aura,
            depth: c.pos_x + c.pos_y - 0.5,
        });
        let (cx, cy) = g.clamp_cell(c.pos_x, c.pos_y);
        let z = weights.z_of(g, g.idx(cx, cy)) + cfg.concepto_lift;
        let (mx, my) = iso.project(c.pos_x, c.pos_y, z);
        quads.push(Quad {
            x: mx - cfg.concepto_size * 0.5,
            y: my - cfg.concepto_size * 0.5,
            w: cfg.concepto_size,
            h: cfg.concepto_size,
            color: cfg.palette.concepto,
            depth: c.pos_x + c.pos_y + 0.75,
        });
    }

    // --- Lemmings: una marca posada sobre el relieve de su celda ---
    let lem = &world.lemmings;
    for i in 0..lem.len() {
        let (px, py) = (lem.pos_x[i], lem.pos_y[i]);
        let (cx, cy) = g.clamp_cell(px, py);
        let z = weights.z_of(g, g.idx(cx, cy)) + cfg.lemming_lift;
        let (sx, sy) = iso.project(px, py, z);
        quads.push(Quad {
            x: sx - cfg.lemming_size * 0.5,
            y: sy - cfg.lemming_size * 0.5,
            w: cfg.lemming_size,
            h: cfg.lemming_size,
            color: cfg.palette.lemming,
            // +0.5 → la marca se pinta después de su celda y de las
            // celdas con su misma diagonal.
            depth: px + py + 0.5,
        });
    }

    // --- Orden de pintor: atrás (depth bajo) primero ---
    quads.sort_by(|a, b| {
        a.depth.partial_cmp(&b.depth).unwrap_or(core::cmp::Ordering::Equal)
    });

    // --- Caja envolvente ---
    let mut plan = RenderPlan { quads, ..Default::default() };
    if let Some(first) = plan.quads.first() {
        plan.min_x = first.x;
        plan.min_y = first.y;
        plan.max_x = first.x + first.w;
        plan.max_y = first.y + first.h;
        for q in &plan.quads {
            plan.min_x = plan.min_x.min(q.x);
            plan.min_y = plan.min_y.min(q.y);
            plan.max_x = plan.max_x.max(q.x + q.w);
            plan.max_y = plan.max_y.max(q.y + q.h);
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iso() -> IsoProjector {
        IsoProjector::new(1.0, 10.0)
    }

    #[test]
    fn empty_world_yields_one_quad_per_cell() {
        let world = World::new(5, 4);
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        assert_eq!(plan.quads.len(), 20);
    }

    #[test]
    fn each_lemming_adds_a_quad() {
        let mut world = World::new(8, 8);
        world.lemmings.spawn(2.0, 3.0, 50.0, [1.0, 0.0, 0.0, 0.0]);
        world.lemmings.spawn(5.0, 5.0, 50.0, [0.0, 1.0, 0.0, 0.0]);
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        // 64 celdas + 2 marcas.
        assert_eq!(plan.quads.len(), 66);
    }

    #[test]
    fn quads_are_depth_sorted_back_to_front() {
        let mut world = World::new(6, 6);
        world.lemmings.spawn(3.0, 3.0, 50.0, [0.0; 4]);
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        for w in plan.quads.windows(2) {
            assert!(w[0].depth <= w[1].depth, "deben ir de atrás hacia adelante");
        }
    }

    #[test]
    fn lemming_draws_after_its_cell() {
        // Lemming en la celda (2,2): su marca (depth 4.5) debe ir tras la
        // celda (2,2) (depth 4.0).
        let mut world = World::new(6, 6);
        world.lemmings.spawn(2.0, 2.0, 50.0, [0.0; 4]);
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        let cfg = PlanConfig::default();
        let marca = plan
            .quads
            .iter()
            .find(|q| q.w == cfg.lemming_size)
            .expect("hay una marca");
        assert_eq!(marca.depth, 4.5);
    }

    #[test]
    fn empty_cell_uses_floor_color() {
        let world = World::new(3, 3);
        let cfg = PlanConfig::default();
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &cfg);
        assert_eq!(plan.quads[0].color, cfg.palette.floor);
    }

    #[test]
    fn high_materia_cell_leans_green() {
        // Sólo la celda (1,1) tiene campo → sólo ella escapa del color
        // `floor`; el resto del tablero queda desnudo.
        let mut world = World::new(3, 3);
        let idx = world.grid.idx(1, 1);
        world.grid.materia[idx] = 100.0;
        let cfg = PlanConfig::default();
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &cfg);
        let painted: Vec<_> = plan
            .quads
            .iter()
            .filter(|q| q.w == cfg.tile && q.color != cfg.palette.floor)
            .collect();
        assert_eq!(painted.len(), 1, "una sola celda con campo");
        assert_eq!(painted[0].color, cfg.palette.materia);
    }

    #[test]
    fn cell_color_blends_two_layers() {
        let mut world = World::new(3, 3);
        let idx = world.grid.idx(0, 0);
        world.grid.materia[idx] = 50.0;
        world.grid.poder[idx] = 50.0;
        let pal = Palette::default();
        let c = cell_color(&world, idx, &pal);
        // Mezcla 50/50 de verde materia y rojo poder → canal por canal.
        for ch in 0..3 {
            let expected = 0.5 * pal.materia[ch] + 0.5 * pal.poder[ch];
            assert!((c[ch] - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn bounding_box_encloses_every_quad() {
        let mut world = World::new(7, 5);
        world.lemmings.spawn(3.0, 2.0, 50.0, [0.0; 4]);
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        for q in &plan.quads {
            assert!(q.x >= plan.min_x - 1e-3);
            assert!(q.y >= plan.min_y - 1e-3);
            assert!(q.x + q.w <= plan.max_x + 1e-3);
            assert!(q.y + q.h <= plan.max_y + 1e-3);
        }
        assert!(plan.width() > 0.0 && plan.height() > 0.0);
    }

    #[test]
    fn plan_is_deterministic() {
        let mut world = World::new(10, 10);
        world.lemmings.spawn(4.0, 6.0, 50.0, [0.5, 0.2, 0.1, 0.7]);
        let idx = world.grid.idx(2, 2);
        world.grid.materia[idx] = 33.0;
        let a = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        let b = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        assert_eq!(a.quads, b.quads);
    }

    #[test]
    fn each_concepto_adds_two_quads_aura_and_marker() {
        use dominium_core::{Concepto, LayerMods};
        let mut world = World::new(8, 8);
        world.conceptos.add(Concepto {
            id: "iglesia".into(),
            sprite_id: 0,
            pos_x: 4.0,
            pos_y: 4.0,
            radius: 2.0,
            mods: LayerMods::default(),
            hack: None,
        });
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        // 64 celdas + 2 quads del concepto (aura + marca).
        assert_eq!(plan.quads.len(), 66);
    }

    #[test]
    fn concepto_marker_paints_after_its_lemming_neighbors() {
        use dominium_core::{Concepto, LayerMods};
        let mut world = World::new(8, 8);
        // Lemming en (4,4), concepto también en (4,4): la marca del concepto
        // (depth 8.75) debe ir tras la marca del lemming (depth 8.5).
        world.lemmings.spawn(4.0, 4.0, 50.0, [0.0; 4]);
        world.conceptos.add(Concepto {
            id: "iglesia".into(),
            sprite_id: 0,
            pos_x: 4.0,
            pos_y: 4.0,
            radius: 1.5,
            mods: LayerMods::default(),
            hack: None,
        });
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        let cfg = PlanConfig::default();
        let lemming_marker_depth = plan
            .quads
            .iter()
            .find(|q| q.w == cfg.lemming_size)
            .expect("hay un lemming")
            .depth;
        let concepto_marker_depth = plan
            .quads
            .iter()
            .find(|q| q.w == cfg.concepto_size)
            .expect("hay una marca de concepto")
            .depth;
        assert!(concepto_marker_depth > lemming_marker_depth);
    }

    #[test]
    fn z_weights_raise_the_terrain() {
        // Con materia alta y peso de relieve, la celda sube (menor y).
        let mut world = World::new(3, 3);
        let idx = world.grid.idx(1, 1);
        world.grid.materia[idx] = 50.0;
        let flat = build_plan(
            &world,
            &iso(),
            &ZWeights { materia: 0.0, ..ZWeights::default() },
            &PlanConfig::default(),
        );
        let raised = build_plan(
            &world,
            &iso(),
            &ZWeights { materia: 1.0, ..ZWeights::default() },
            &PlanConfig::default(),
        );
        let cfg = PlanConfig::default();
        // La celda (1,1) es la única con campo → la única coloreada
        // `materia`; la identificamos por color, no por `depth`.
        let pick = |p: &RenderPlan| {
            p.quads
                .iter()
                .find(|q| q.w == cfg.tile && q.color == cfg.palette.materia)
                .unwrap()
                .y
        };
        assert!(pick(&raised) < pick(&flat), "el relieve sube la celda");
    }
}
