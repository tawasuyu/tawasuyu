//! Transiciones de estado del modelo: avance del tick, reseed, ring de
//! snapshots y trails, clusters k-means y el overlay de trayectorias.
//! También los pequeños helpers de mutación del `Model` que usan `update`.

use dominium_core::{kmeans_psi, Concepto, LayerMods, World};
use dominium_render_plan::{Color, Quad, RenderMode, RenderPlan};
use dominium_physics::tick;

use crate::consts::{GRID, KMEANS_REFRESH_TICKS, SNAPSHOT_RING_CAP, TRAIL_CAP};
use crate::model::Model;
use crate::worldgen::seed;

/// Accede mutable al Concepto seleccionado, si lo hay.
pub(crate) fn selected_mut(m: &mut Model) -> Option<&mut Concepto> {
    let i = m.selected?;
    m.world.conceptos.items.get_mut(i)
}

/// Agrega un Concepto en `(x, y)` (clamp al grid), lo nombra
/// `nuevo-N` y queda seleccionado para edición inmediata.
pub(crate) fn spawn_concepto_at(m: &mut Model, x: f32, y: f32) {
    let max = (GRID as f32) - 1.0;
    let n = m.world.conceptos.len();
    let new = Concepto {
        id: format!("nuevo-{}", n + 1),
        sprite_id: 0,
        pos_x: x.clamp(0.0, max),
        pos_y: y.clamp(0.0, max),
        radius: 4.0,
        mods: LayerMods::default(),
        hack: None,
        persuasion: None,
    };
    let i = m.world.conceptos.add(new);
    m.selected = Some(i);
}

/// Copia `ZWeights` (relieve visual) al array `[f32; 5]` que SimParams
/// usa como relieve físico, manteniendo el orden de capas del `Grid`.
pub(crate) fn mirror_zweights_to_relieve(
    z: &dominium_iso::ZWeights,
    relieve: &mut [f32; 5],
) {
    relieve[dominium_core::RELIEVE_MATERIA] = z.materia;
    relieve[dominium_core::RELIEVE_PSIQUE] = z.psique;
    relieve[dominium_core::RELIEVE_PODER] = z.poder;
    relieve[dominium_core::RELIEVE_ORO] = z.oro;
    relieve[dominium_core::RELIEVE_DEGRADACION] = z.degradacion;
}

/// Un paso de simulación; re-siembra si la población colapsa. Captura
/// también el snapshot del estado y el frame de trails (después de avanzar,
/// así el "presente" siempre coincide con `world`).
pub(crate) fn advance(m: &mut Model) {
    tick(&mut m.world, &m.params);
    m.tick += 1;
    if m.world.lemmings.is_empty() {
        m.epoch += 1;
        m.rng_seed = m
            .rng_seed
            .wrapping_mul(2862933555777941757)
            .wrapping_add(1);
        m.world = seed(m.rng_seed);
        m.tick = 0;
        m.snapshots.clear();
        m.trails.clear();
        m.cluster_assignments.clear();
    }
    push_snapshot(m);
    push_trail_frame(m);
    // K-means de psi: sólo cuando el render lo necesita. Si el usuario
    // está en otro modo, no pagamos el costo.
    if matches!(m.cfg.render_mode, RenderMode::PsiCluster)
        && m.tick.saturating_sub(m.cluster_last_refresh) >= KMEANS_REFRESH_TICKS
    {
        refresh_clusters(m);
    }
}

/// Tres colores fijos del paleta de clusters — orden de aparición en el
/// resultado de `kmeans_psi`. Magenta / cian / amarillo: los más fáciles de
/// distinguir sobre cualquier fondo de bioma.
pub(crate) const CLUSTER_COLORS: [Color; 3] = [
    [0.96, 0.30, 0.72, 1.0], // magenta
    [0.30, 0.90, 0.90, 1.0], // cian
    [0.96, 0.92, 0.30, 1.0], // amarillo
];

/// Recalcula `cluster_assignments` desde el `World` actual y deja el tick
/// como timestamp del refresh. Si `kmeans_psi` devuelve `None` (pob < K),
/// limpia las asignaciones para que los lemmings caigan al color default.
pub(crate) fn refresh_clusters(m: &mut Model) {
    if let Some(km) = kmeans_psi(&m.world) {
        m.cluster_assignments = km.assignments;
    } else {
        m.cluster_assignments.clear();
    }
    m.cluster_last_refresh = m.tick;
}

/// Color para el lemming `i` según el `RenderMode` actual y las
/// asignaciones de cluster vigentes. Se usa como override de
/// `build_plan_with_overrides`.
pub(crate) fn lemming_color_for(m: &Model, i: usize) -> Color {
    if matches!(m.cfg.render_mode, RenderMode::PsiCluster)
        && i < m.cluster_assignments.len()
    {
        let c = m.cluster_assignments[i] as usize;
        if c < CLUSTER_COLORS.len() {
            return CLUSTER_COLORS[c];
        }
    }
    m.cfg.palette.lemming
}

pub(crate) fn reseed(m: &mut Model) {
    m.rng_seed = m.rng_seed.wrapping_add(0x9E37_79B9);
    m.world = seed(m.rng_seed);
    m.tick = 0;
    m.epoch += 1;
    m.snapshots.clear();
    m.trails.clear();
    m.rewind_offset = 0;
}

/// Empuja el `World` actual al ring (clone barato: SoA + Vec). Drop del más
/// viejo al exceder la capacidad.
pub(crate) fn push_snapshot(m: &mut Model) {
    if m.snapshots.len() == SNAPSHOT_RING_CAP {
        m.snapshots.pop_front();
    }
    m.snapshots.push_back(m.world.clone());
}

/// Empuja el frame de posiciones de todos los lemmings vivos al ring.
pub(crate) fn push_trail_frame(m: &mut Model) {
    if m.trails.len() == TRAIL_CAP {
        m.trails.pop_front();
    }
    let lem = &m.world.lemmings;
    let frame: Vec<(f32, f32)> = (0..lem.len())
        .map(|i| (lem.pos_x[i], lem.pos_y[i]))
        .collect();
    m.trails.push_back(frame);
}

/// Devuelve el `World` que actualmente se está mostrando — el presente
/// (`world`) si no hay rewind, o el snapshot apropiado si lo hay.
pub(crate) fn displayed_world(m: &Model) -> &World {
    if m.rewind_offset == 0 || m.snapshots.is_empty() {
        &m.world
    } else {
        let len = m.snapshots.len();
        let idx = len.saturating_sub(1 + m.rewind_offset);
        &m.snapshots[idx]
    }
}

/// Pinta las posiciones históricas de los lemmings como quads diminutos
/// con alpha decreciente — los más viejos casi transparentes. Va después
/// del `build_plan` para que los trails queden por encima del suelo pero
/// por debajo del HUD; depth pequeño constante negativo para no romper el
/// orden de pintor de las celdas.
///
/// Se llama sólo en vivo (no en rewind), porque en rewind el `World` que
/// se renderiza no necesariamente tiene los mismos índices de lemming que
/// el frame de trails — y mezclarlos confundiría al ojo más que ayudar.
pub(crate) fn overlay_trails(plan: &mut RenderPlan, m: &Model) {
    let n_frames = m.trails.len();
    if n_frames == 0 {
        return;
    }
    let lemming_color = m.cfg.palette.lemming;
    // Tamaño de la moteta: la mitad del marker del lemming, así no compite
    // visualmente con la posición actual.
    let size = m.cfg.lemming_size * 0.45;
    for (k, frame) in m.trails.iter().enumerate() {
        // k=0 es el más viejo → alpha bajo; k=n-1 el más nuevo → alpha alto.
        // No incluyo el último frame: ya está pintado por el lemming actual.
        if k + 1 == n_frames {
            break;
        }
        let t = (k + 1) as f32 / n_frames as f32; // ∈ (0, 1)
        let alpha = 0.10 + 0.40 * t;
        let color: Color = [
            lemming_color[0],
            lemming_color[1],
            lemming_color[2],
            alpha,
        ];
        for &(x, y) in frame {
            let (sx, sy) = m.iso.project(x, y, m.cfg.lemming_lift * 0.5);
            plan.quads.push(Quad {
                x: sx - size * 0.5,
                y: sy - size * 0.5,
                w: size,
                h: size,
                color,
                // Detrás de los Lemmings vivos (que pintan a depth ≈ x+y+0.5)
                // pero delante de la celda (depth x+y).
                depth: x + y + 0.25,
            });
        }
    }
    // Mantengo el plan ordenado: insert al final desordena. Re-ordeno por
    // depth — coste O(N log N) pero N es del orden de 50·24 = 1200 quads.
    plan.quads.sort_by(|a, b| {
        a.depth.partial_cmp(&b.depth).unwrap_or(std::cmp::Ordering::Equal)
    });
    // Re-extender la bounding box por si los trails caen fuera.
    for q in &plan.quads {
        plan.min_x = plan.min_x.min(q.x);
        plan.min_y = plan.min_y.min(q.y);
        plan.max_x = plan.max_x.max(q.x + q.w);
        plan.max_y = plan.max_y.max(q.y + q.h);
    }
}
