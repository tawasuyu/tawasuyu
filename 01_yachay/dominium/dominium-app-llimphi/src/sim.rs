//! Helpers de mutación/render que tocan estado de VISTA además del dominio.
//! El ciclo de vida de la simulación (avance, reseed, snapshots, trails,
//! clusters) vive en `dominium_sim::Sim` (regla #2); acá quedan los que
//! cruzan con `selected`, `cfg`, `iso` u otros campos del frontend.

use dominium_core::{Concepto, LayerMods};
use dominium_render_plan::{Color, Quad, RenderMode, RenderPlan};

use crate::consts::GRID;
use crate::model::Model;

/// Huella barata del estado que determina el `RenderPlan`. Si dos frames
/// tienen la misma huella, el plan es idéntico y se puede reusar el cacheado
/// en vez de re-iterar las 57 600 celdas (ver `Model::plan_cache`).
///
/// Incluye TODO lo que `build_plan_with_overrides` + `overlay_trails` leen:
/// el reloj de la sim (cubre cualquier avance del mundo), los conceptos
/// (posición/forma/sprite/mods que editás), el relieve visual (`weights`),
/// la config de presentación (`cfg`: tile/modo/textura/andina/luz), el
/// override de color por cluster (sólo en PsiCluster) y el toggle de trails.
/// NO incluye `selected` (la selección no toca el plan, sólo el panel).
///
/// Cuando la sim corre, `tick` cambia cada frame → la huella cambia → se
/// reconstruye (correcto: el mundo cambió). Cuando está pausada o sólo se
/// mueve la UI, la huella se mantiene → cache hit → costo de build ≈ 0.
pub(crate) fn render_fingerprint(m: &Model) -> u64 {
    // Mezclador FNV-1a de 64 bits: barato y con buena dispersión para esta
    // colección heterogénea de bits.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |x: u64| {
        h ^= x;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    let f = |v: f32| v.to_bits() as u64;

    // Reloj de la sim — cubre cualquier mutación del World por `advance`.
    // Se usa el mundo MOSTRADO (rewind se mueve por `rewind_offset`).
    mix(m.sim.tick);
    mix(m.sim.epoch);
    mix(m.sim.rewind_offset as u64);

    // Relieve visual.
    mix(f(m.weights.materia));
    mix(f(m.weights.psique));
    mix(f(m.weights.poder));
    mix(f(m.weights.oro));
    mix(f(m.weights.degradacion));

    // Cámara/escala + pan. SIN el pan, panear no cambiaría la huella y la
    // caché del plan no se invalidaría → la cámara quedaría congelada (modo
    // de falla ya visto en este frente).
    mix(f(m.iso.scale));
    mix(f(m.iso.z_factor));
    mix(f(m.pan.0));
    mix(f(m.pan.1));

    // Config de presentación.
    mix(f(m.cfg.tile));
    mix(f(m.cfg.lemming_size));
    mix(f(m.cfg.lemming_lift));
    mix(f(m.cfg.concepto_size));
    mix(f(m.cfg.concepto_lift));
    mix(f(m.cfg.light_dir.0));
    mix(f(m.cfg.light_dir.1));
    mix(m.cfg.andina_layers as u64);
    mix(f(m.cfg.andina_threshold));
    mix(m.cfg.texture as u64);
    mix(match m.cfg.render_mode {
        RenderMode::Composite => 0,
        RenderMode::Heatmap(l) => 1 + l as u64 * 16,
        RenderMode::PsiCluster => 0xF00D,
    });

    // Toggle de trails (sólo afecta en vivo).
    mix(m.show_trails as u64);

    // Conceptos: posición/forma/sprite/mods de cada uno (lo que el plan dibuja).
    mix(m.sim.world.conceptos.len() as u64);
    for c in &m.sim.world.conceptos.items {
        mix(f(c.pos_x));
        mix(f(c.pos_y));
        mix(f(c.radius));
        mix(c.sprite_id as u64);
        mix(f(c.mods.materia));
        mix(f(c.mods.psique));
        mix(f(c.mods.poder));
        mix(f(c.mods.oro));
    }

    // En PsiCluster el color de cada lemming viene del k-means; los refrescos
    // ocurren dentro de `advance` (cubierto por tick) o en `refresh_clusters`
    // al cambiar de modo, pero el contenido del vector puede mudar sin tocar
    // tick — lo mezclamos para no perder un refresh.
    if matches!(m.cfg.render_mode, RenderMode::PsiCluster) {
        mix(m.sim.cluster_assignments.len() as u64);
        for chunk in m.sim.cluster_assignments.chunks(8) {
            let mut word = 0u64;
            for (k, &b) in chunk.iter().enumerate() {
                word |= (b as u64) << (k * 8);
            }
            mix(word);
        }
    }

    h
}

/// Accede mutable al Concepto seleccionado, si lo hay.
pub(crate) fn selected_mut(m: &mut Model) -> Option<&mut Concepto> {
    let i = m.selected?;
    m.sim.world.conceptos.items.get_mut(i)
}

/// Agrega un Concepto en `(x, y)` (clamp al grid), lo nombra
/// `nuevo-N` y queda seleccionado para edición inmediata.
pub(crate) fn spawn_concepto_at(m: &mut Model, x: f32, y: f32) {
    let max = (GRID as f32) - 1.0;
    let n = m.sim.world.conceptos.len();
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
    let i = m.sim.world.conceptos.add(new);
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

/// Tres colores fijos del paleta de clusters — orden de aparición en el
/// resultado de `kmeans_psi`. Magenta / cian / amarillo: los más fáciles de
/// distinguir sobre cualquier fondo de bioma.
pub(crate) const CLUSTER_COLORS: [Color; 3] = [
    [0.96, 0.30, 0.72, 1.0], // magenta
    [0.30, 0.90, 0.90, 1.0], // cian
    [0.96, 0.92, 0.30, 1.0], // amarillo
];

/// Color para el lemming `i` según el `RenderMode` actual y las
/// asignaciones de cluster vigentes. Se usa como override de
/// `build_plan_with_overrides`.
pub(crate) fn lemming_color_for(m: &Model, i: usize) -> Color {
    if matches!(m.cfg.render_mode, RenderMode::PsiCluster)
        && i < m.sim.cluster_assignments.len()
    {
        let c = m.sim.cluster_assignments[i] as usize;
        if c < CLUSTER_COLORS.len() {
            return CLUSTER_COLORS[c];
        }
    }
    m.cfg.palette.lemming
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
    let n_frames = m.sim.trails.len();
    if n_frames == 0 {
        return;
    }
    let lemming_color = m.cfg.palette.lemming;
    // Tamaño de la moteta: la mitad del marker del lemming, así no compite
    // visualmente con la posición actual.
    let size = m.cfg.lemming_size * 0.45;
    for (k, frame) in m.sim.trails.iter().enumerate() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Model, PanelTab};
    use dominium_core::SimParams;
    use dominium_iso::{IsoProjector, ZWeights};
    use dominium_render_plan::PlanConfig;
    use dominium_sim::Sim;
    use llimphi_theme::Theme;
    use llimphi_widget_text_input::TextInputState;

    /// Model mínimo para tests headless de helpers puros (huella de render).
    /// Mundo pequeño, cámara default, sin watcher ni menús abiertos.
    fn test_model() -> Model {
        let world = dominium_core::worldgen::seed(0x1234, 8, 16, dominium_core::Conceptos::new());
        let sim = Sim::new(
            world,
            SimParams::default(),
            0x1234,
            8,
            8,
            30,
            false,
            Box::new(|s| dominium_core::worldgen::seed(s, 8, 16, dominium_core::Conceptos::new())),
        );
        Model {
            sim,
            controller: None,
            setpoint: 600.0,
            iso: IsoProjector::new(3.0, 0.55),
            pan: (0.0, 0.0),
            weights: ZWeights::default(),
            panel_scroll: 0.0,
            cfg: PlanConfig { render_mode: RenderMode::Composite, ..PlanConfig::default() },
            selected: None,
            sync_relieve: false,
            id_input: TextInputState::new(),
            id_input_focused: false,
            scenario_idx: 0,
            show_trails: false,
            theme: Theme::dark(),
            _wawa_watcher: None,
            panel_tab: PanelTab::Mundo,
            onboarding_done: true,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: llimphi_motion::Tween::idle(1.0),
            edit_menu: None,
            edit_active: usize::MAX,
            edit_anim: llimphi_motion::Tween::idle(1.0),
            clipboard: llimphi_clipboard::SystemClipboard::new(),
            plan_cache: std::cell::RefCell::new(None),
        }
    }

    /// Cambiar el pan de cámara cambia la huella de render → invalida la
    /// caché del plan (sin esto, panear no repintaría: cámara congelada).
    #[test]
    fn fingerprint_changes_with_pan() {
        let mut m = test_model();
        let fp0 = render_fingerprint(&m);
        m.pan = (200.0, -120.0);
        let fp1 = render_fingerprint(&m);
        assert_ne!(fp0, fp1, "el pan debe cambiar la huella de render");

        // Mover sólo Y también cambia (las dos componentes se mezclan).
        m.pan = (0.0, 50.0);
        let fp2 = render_fingerprint(&m);
        assert_ne!(fp0, fp2, "pan.1 solo debe cambiar la huella");

        // Volver a (0,0) restaura la huella original (la mezcla es función
        // pura del estado).
        m.pan = (0.0, 0.0);
        assert_eq!(render_fingerprint(&m), fp0, "pan (0,0) restaura la huella");
    }

    /// El zoom (iso.scale) también cambia la huella — ya estaba cubierto,
    /// pero lo fijamos junto al pan para que ambos ejes de cámara queden
    /// blindados contra una regresión de caché.
    #[test]
    fn fingerprint_changes_with_zoom() {
        let mut m = test_model();
        let fp0 = render_fingerprint(&m);
        m.iso = IsoProjector::new(6.0, 0.55);
        assert_ne!(fp0, render_fingerprint(&m), "el zoom debe cambiar la huella");
    }
}
