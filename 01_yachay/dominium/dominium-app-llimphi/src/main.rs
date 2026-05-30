//! `dominium-app-llimphi` — la ventana viva del simulador sobre
//! Llimphi.
//!
//! Compone la cadena agnóstica de dominium con el canvas Llimphi:
//!
//! ```text
//!   dominium-core ─► dominium-physics ─► dominium-iso ─►
//!   dominium-render-plan ─► dominium-canvas-llimphi ─► [esta ventana]
//! ```
//!
//! Un loop de fondo (~11 Hz) avanza la simulación y reentra al
//! `update` vía `Handle::dispatch(Msg::Tick)`. Cuando la población
//! colapsa, el mundo se re-siembra solo. El panel derecho muestra
//! stats y dos controles (play/pausa, re-sembrar).
//!
//! El crate está partido en módulos: `consts` (constantes de mundo),
//! `model` (Model + Msg + enums de edición), `packs` (scenarios embebidos
//! + persistencia), `worldgen` (PRNG + ruido + `seed`), `sim` (transiciones
//! + helpers de mutación) y `view` (todas las vistas). Acá queda sólo el
//! `impl App` que las orquesta.

mod consts;
mod model;
mod packs;
mod sim;
mod view;
mod worldgen;

use std::collections::VecDeque;
use std::time::Duration;

use dominium_core::{BehaviorHack, Conceptos, PsiMetrics, SimParams, Trigger, WorldStats};
use dominium_iso::{IsoProjector, ZWeights};
use dominium_render_plan::{build_plan_with_overrides, PlanConfig, RenderLayer, RenderMode};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    length, percent, Dimension, FlexDirection, Size, Style,
};
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_text_input::TextInputState;
use wawa_config_llimphi::theme_from_wawa;

use crate::consts::{GRID, SNAPSHOT_RING_CAP, TICK_MS, TRAIL_CAP};
use crate::model::{Layer, Model, Msg, PanelTab, ParamSlot, ZSlot};
use crate::packs::{default_conceptos, load_user_pack, save_user_pack, scenario_packs};
use crate::sim::{
    advance, displayed_world, lemming_color_for, mirror_zweights_to_relieve, overlay_trails,
    refresh_clusters, reseed, selected_mut, spawn_concepto_at,
};
use crate::view::{canvas_pane, onboarding_bar, side_panel, status_bar};
use crate::worldgen::{bioma_palette, seed};

struct Dominium;

impl App for Dominium {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "dominium · campo medio (llimphi)"
    }

    fn initial_size() -> (u32, u32) {
        (1120, 720)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        // Loop de tick a ~11 Hz; el handle ya sabe cómo dejar morir
        // el thread cuando el event loop se cierre.
        handle.spawn_periodic(Duration::from_millis(TICK_MS), || Msg::Tick);

        // Bus de configuración del SO. Theme y locale arrancan desde
        // el archivo si existe; el watcher reentra al `update` cuando
        // cambia.
        let wawa_cfg = wawa_config::WawaConfig::load();
        let theme = theme_from_wawa(&wawa_cfg, &Theme::dark());
        let _ = rimay_localize::set_locale(&wawa_cfg.lang);
        let handle_clone = handle.clone();
        let wawa_watcher = wawa_config::ConfigWatcher::spawn(move |new_cfg| {
            handle_clone.dispatch(Msg::WawaConfigChanged(Box::new(new_cfg)));
        })
        .map_err(|e| eprintln!("dominium · wawa-config watcher: {e}"))
        .ok();

        let rng_seed = 0xD0_31_31_07;
        // SimParams con overrides puntuales. La iteración anterior puso
        // `metabolic_cost=0.35` que con energía inicial 40-80 vacía a los
        // lemmings en ~200 ticks → murieron todos en 10s. Acá la calibración
        // afloja: drenaje basal modesto, threshold de réplica más bajo, hijos
        // que arrancan con energía digna. La capacidad de carga sigue
        // limitada por el regrowth + carrying_capacity, no por matar al
        // adulto promedio.
        let params = SimParams {
            // Difusión y entropía bajas → la psique de los mares no se
            // empuja a tierra en pocos ticks. (Defaults son 0.1 / 0.01.)
            diffusion_rate: 0.02,
            entropy_rate: 0.004,
            // Regrowth limitado a la carga base de la llanura — sin esto
            // el regrowth llena de materia incluso los mares (que tienen
            // 0 inicial pero materia → carrying_capacity).
            regrowth_rate: 0.004,
            carrying_capacity: 40.0,
            // Drenaje basal mínimo: 0.05 E/tick frena el techo sin matar
            // la cohorte joven (energía inicial 40-80 dura > 800 ticks).
            metabolic_cost: 0.05,
            // Réplica menos cara → la sociedad alcanza un equilibrio
            // dinámico en vez de extinguirse.
            replicate_threshold: 28.0,
            child_energy_frac: 0.45,
            abundance_threshold: 50.0,
            ..SimParams::default()
        };
        Model {
            world: seed(rng_seed),
            params,
            // Scale 3.0 para que la grilla 240×240 entre en pantalla. z_factor
            // 0.55 levanta el relieve a algo perceptible sin que los picos
            // exploten: mares ~−25 px, llanura plana, colinas ~+10 px,
            // picos ~+30 px. La versión 0.35 anterior daba total ~9 px → el
            // mapa parecía plano por completo.
            iso: IsoProjector::new(3.0, 0.55),
            // Relieve por bioma, recalibrado para valores nuevos:
            //   - mares  → z ≈ −15 (psique 200 × −0.075)
            //   - llanura → z ≈ +1.6 (materia 80 × 0.02)
            //   - colinas → z ≈ +6 (poder 15 × 0.4)
            //   - picos  → z ≈ +21 (degradacion 16 × 1.3 + el resto)
            weights: ZWeights {
                materia: 0.02,
                psique: -0.075,
                poder: 0.40,
                oro: 0.0,
                degradacion: 1.30,
            },
            cfg: PlanConfig {
                tile: 3.0,
                lemming_size: 2.6,
                lemming_lift: 0.6,
                concepto_size: 7.0,
                concepto_lift: 2.0,
                light_dir: (0.55, 0.35),
                andina_layers: 0,
                andina_threshold: 1.0,
                palette: bioma_palette(),
                render_mode: RenderMode::Composite,
                // Textura procedural OFF por default: con miles de celdas,
                // los micro-quads empiezan a tapar la maqueta y el render
                // pierde claridad. El usuario lo prende en el tab Vista
                // si quiere "estampa".
                texture: false,
            },
            running: true,
            tick: 0,
            epoch: 0,
            rng_seed,
            selected: None,
            sync_relieve: false,
            id_input: TextInputState::new(),
            id_input_focused: false,
            scenario_idx: 0,
            snapshots: VecDeque::with_capacity(SNAPSHOT_RING_CAP),
            rewind_offset: 0,
            trails: VecDeque::with_capacity(TRAIL_CAP),
            show_trails: false,
            theme,
            _wawa_watcher: wawa_watcher,
            cluster_assignments: Vec::new(),
            cluster_last_refresh: 0,
            panel_tab: PanelTab::Mundo,
            onboarding_done: false,
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                // Si el usuario está revisando el pasado, la sim queda
                // congelada para no acumular divergencia con el ring.
                if m.running && m.rewind_offset == 0 {
                    advance(&mut m);
                }
            }
            Msg::TogglePlay => {
                m.running = !m.running;
            }
            Msg::Reseed => {
                reseed(&mut m);
            }
            Msg::LimpiarConceptos => {
                m.world.conceptos.clear();
                // Romper los hack_locks vivos: sin Concepto que los sostenga,
                // los lemmings vuelven a la lógica normal.
                for lock in m.world.lemmings.hack_lock.iter_mut() {
                    *lock = 0;
                }
                m.selected = None;
            }
            Msg::SembrarConceptos => {
                m.world.conceptos = default_conceptos();
                m.selected = None;
            }
            Msg::SelectConcepto(i) => {
                if i < m.world.conceptos.len() {
                    m.selected = Some(i);
                }
            }
            Msg::DeselectConcepto => m.selected = None,
            Msg::EditMod(layer, dv) => {
                if let Some(i) = m.selected {
                    if let Some(c) = m.world.conceptos.items.get_mut(i) {
                        let slot = match layer {
                            Layer::Materia => &mut c.mods.materia,
                            Layer::Psique => &mut c.mods.psique,
                            Layer::Poder => &mut c.mods.poder,
                            Layer::Oro => &mut c.mods.oro,
                        };
                        *slot = (*slot + dv).clamp(-1.0, 1.0);
                    }
                }
            }
            Msg::EditRadius(dv) => {
                if let Some(i) = m.selected {
                    if let Some(c) = m.world.conceptos.items.get_mut(i) {
                        c.radius = (c.radius + dv).clamp(0.5, 20.0);
                    }
                }
            }
            Msg::DeleteSelected => {
                if let Some(i) = m.selected.take() {
                    if i < m.world.conceptos.len() {
                        m.world.conceptos.remove(i);
                        for lock in m.world.lemmings.hack_lock.iter_mut() {
                            *lock = 0;
                        }
                    }
                }
            }
            Msg::EditParam(slot, dv) => {
                let (lo, hi) = slot.range();
                match slot {
                    ParamSlot::ClimbCost => {
                        m.params.climb_cost = (m.params.climb_cost + dv).clamp(lo, hi)
                    }
                    ParamSlot::DiffusionRate => {
                        m.params.diffusion_rate =
                            (m.params.diffusion_rate + dv).clamp(lo, hi)
                    }
                    ParamSlot::EntropyRate => {
                        m.params.entropy_rate = (m.params.entropy_rate + dv).clamp(lo, hi)
                    }
                    ParamSlot::MoveCost => {
                        m.params.move_cost = (m.params.move_cost + dv).clamp(lo, hi)
                    }
                    ParamSlot::SeasonPeriod => {
                        let v = (m.params.season_period as f32 + dv).clamp(lo, hi);
                        m.params.season_period = v as u32;
                    }
                    ParamSlot::SeasonAmplitude => {
                        m.params.season_amplitude =
                            (m.params.season_amplitude + dv).clamp(lo, hi)
                    }
                    ParamSlot::PsiModulation => {
                        m.params.psi_effect_modulation =
                            (m.params.psi_effect_modulation + dv).clamp(lo, hi)
                    }
                    ParamSlot::SocialRadius => {
                        m.params.social_radius =
                            (m.params.social_radius + dv).clamp(lo, hi)
                    }
                    ParamSlot::ContagionRate => {
                        m.params.contagion_rate =
                            (m.params.contagion_rate + dv).clamp(lo, hi)
                    }
                    ParamSlot::HomophilyThreshold => {
                        m.params.homophily_threshold =
                            (m.params.homophily_threshold + dv).clamp(lo, hi)
                    }
                }
            }
            Msg::EditZWeight(slot, dv) => {
                let s = match slot {
                    ZSlot::Materia => &mut m.weights.materia,
                    ZSlot::Psique => &mut m.weights.psique,
                    ZSlot::Poder => &mut m.weights.poder,
                    ZSlot::Oro => &mut m.weights.oro,
                    ZSlot::Degradacion => &mut m.weights.degradacion,
                };
                *s = (*s + dv).clamp(-2.0, 2.0);
                if m.sync_relieve {
                    mirror_zweights_to_relieve(&m.weights, &mut m.params.relieve);
                }
            }
            Msg::GuardarPack => save_user_pack(&m.world.conceptos),
            Msg::CargarPack => {
                if let Some(cs) = load_user_pack() {
                    m.world.conceptos = cs;
                    for lock in m.world.lemmings.hack_lock.iter_mut() {
                        *lock = 0;
                    }
                    m.selected = None;
                }
            }
            Msg::CrearConcepto => {
                let center = (GRID as f32) * 0.5;
                spawn_concepto_at(&mut m, center, center);
            }
            Msg::CanvasClick(wx, wy) => {
                // Primer click sobre el canvas también apaga el hint de
                // onboarding — si llegó hasta acá, ya entendió que se
                // puede interactuar con el mapa.
                m.onboarding_done = true;
                // Hit-test contra Conceptos existentes (centro + radio
                // pickeable acotado). Si pega, selecciona sin crear; si
                // no, crea un Concepto nuevo ahí.
                let mut hit: Option<usize> = None;
                for (i, c) in m.world.conceptos.items.iter().enumerate() {
                    let dx = wx - c.pos_x;
                    let dy = wy - c.pos_y;
                    let pick_r = c.radius.min(3.0);
                    if dx * dx + dy * dy <= pick_r * pick_r {
                        hit = Some(i);
                        break;
                    }
                }
                match hit {
                    Some(i) => m.selected = Some(i),
                    None => spawn_concepto_at(&mut m, wx, wy),
                }
            }
            Msg::ToggleSyncRelieve => {
                m.sync_relieve = !m.sync_relieve;
                if m.sync_relieve {
                    mirror_zweights_to_relieve(&m.weights, &mut m.params.relieve);
                }
            }
            Msg::ToggleAndina => {
                // 0 ↔ 3 capas. El threshold no cambia.
                m.cfg.andina_layers = if m.cfg.andina_layers == 0 { 3 } else { 0 };
            }
            Msg::HackToggle => {
                if let Some(c) = selected_mut(&mut m) {
                    c.hack = match c.hack {
                        Some(_) => None,
                        None => Some(BehaviorHack {
                            trigger: Trigger::Always,
                            forced_action: 2, // Sincronizar — el default más visible
                            duration: 30,
                        }),
                    };
                }
            }
            Msg::HackCycleTrigger => {
                if let Some(c) = selected_mut(&mut m) {
                    if let Some(h) = c.hack.as_mut() {
                        h.trigger = match h.trigger {
                            Trigger::Always => Trigger::EnergiaBajo(15.0),
                            Trigger::EnergiaBajo(_) => Trigger::EdadSobre(100),
                            Trigger::EdadSobre(_) => Trigger::Always,
                        };
                    }
                }
            }
            Msg::HackCycleAction => {
                if let Some(c) = selected_mut(&mut m) {
                    if let Some(h) = c.hack.as_mut() {
                        h.forced_action = (h.forced_action + 1) % 6;
                    }
                }
            }
            Msg::HackEditTriggerParam(dv) => {
                if let Some(c) = selected_mut(&mut m) {
                    if let Some(h) = c.hack.as_mut() {
                        h.trigger = match h.trigger {
                            Trigger::Always => Trigger::Always,
                            Trigger::EnergiaBajo(v) => {
                                Trigger::EnergiaBajo((v + dv).clamp(0.0, 100.0))
                            }
                            Trigger::EdadSobre(v) => {
                                let next = (v as f32 + dv).clamp(0.0, 1000.0);
                                Trigger::EdadSobre(next as u32)
                            }
                        };
                    }
                }
            }
            Msg::HackEditDuration(dv) => {
                if let Some(c) = selected_mut(&mut m) {
                    if let Some(h) = c.hack.as_mut() {
                        let next = (h.duration as f32 + dv).clamp(1.0, 500.0);
                        h.duration = next as u32;
                    }
                }
            }
            Msg::CycleSprite => {
                if let Some(c) = selected_mut(&mut m) {
                    // 0 (sin glifo) → 1..=SPRITE_COUNT → 0 ...
                    c.sprite_id = (c.sprite_id + 1) % (dominium_render_plan::SPRITE_COUNT + 1);
                }
            }
            Msg::CanvasDragMove(dwx, dwy) => {
                if let Some(c) = selected_mut(&mut m) {
                    let max = (GRID as f32) - 1.0;
                    c.pos_x = (c.pos_x + dwx).clamp(0.0, max);
                    c.pos_y = (c.pos_y + dwy).clamp(0.0, max);
                }
            }
            Msg::FocusIdInput => {
                if let Some(c) = m.selected.and_then(|i| m.world.conceptos.items.get(i)) {
                    m.id_input.set_text(c.id.clone());
                    m.id_input_focused = true;
                }
            }
            Msg::BlurIdInput => {
                m.id_input_focused = false;
            }
            Msg::IdInputKey(ev) => {
                if m.id_input_focused && m.id_input.apply_key(&ev) {
                    let new_id = m.id_input.text().to_string();
                    if let Some(c) = selected_mut(&mut m) {
                        c.id = new_id;
                    }
                }
            }
            Msg::CycleScenario => {
                let n = scenario_packs().len();
                m.scenario_idx = (m.scenario_idx + 1) % n;
            }
            Msg::LoadScenario => {
                let packs = scenario_packs();
                let (_, json) = packs[m.scenario_idx];
                if let Ok(cs) = serde_json::from_str::<Conceptos>(json) {
                    m.world.conceptos = cs;
                    for lock in m.world.lemmings.hack_lock.iter_mut() {
                        *lock = 0;
                    }
                    m.selected = None;
                }
            }
            Msg::CycleRenderMode => {
                m.cfg.render_mode = match m.cfg.render_mode {
                    RenderMode::Composite => RenderMode::Heatmap(RenderLayer::Materia),
                    RenderMode::Heatmap(RenderLayer::Degradacion) => RenderMode::PsiCluster,
                    RenderMode::Heatmap(l) => RenderMode::Heatmap(l.next()),
                    RenderMode::PsiCluster => RenderMode::Composite,
                };
                // Forzar refresh inmediato del k-means al entrar al modo.
                if matches!(m.cfg.render_mode, RenderMode::PsiCluster) {
                    refresh_clusters(&mut m);
                }
            }
            Msg::ToggleTrails => {
                m.show_trails = !m.show_trails;
            }
            Msg::ToggleTexture => {
                m.cfg.texture = !m.cfg.texture;
            }
            Msg::RewindBy(dv) => {
                let cap = m.snapshots.len().saturating_sub(1);
                let cur = m.rewind_offset as f32;
                let next = (cur + dv).clamp(0.0, cap as f32);
                m.rewind_offset = next as usize;
            }
            Msg::RewindHome => {
                m.rewind_offset = 0;
            }
            Msg::ToggleBigFive => {
                m.params.big_five = !m.params.big_five;
                if m.params.big_five {
                    // Saves Big Four que entraron sin columna psi5 hay que
                    // rellenarlos antes de que el motor consulte
                    // `lemmings.psi5[i]`.
                    m.world.lemmings.ensure_psi5_len();
                }
            }
            Msg::CyclePsiPolicy => {
                m.params.action_policy = match m.params.action_policy {
                    dominium_core::ActionPolicy::Fixed => {
                        if m.params.policy_reeval_period == 0 {
                            m.params.policy_reeval_period = 20;
                        }
                        dominium_core::ActionPolicy::PsiArgmax
                    }
                    dominium_core::ActionPolicy::PsiArgmax => {
                        dominium_core::ActionPolicy::Fixed
                    }
                };
            }
            Msg::WawaConfigChanged(cfg) => {
                // Re-armamos el theme y el locale. El locale lo respeta
                // el próximo `view()` porque `rimay_localize::t(...)` se
                // re-llama cada frame.
                m.theme = theme_from_wawa(&cfg, &m.theme);
                if cfg.lang != rimay_localize::current_locale() {
                    let _ = rimay_localize::set_locale(&cfg.lang);
                }
            }
            Msg::SelectTab(tab) => {
                m.panel_tab = tab;
            }
            Msg::DismissOnboarding => {
                m.onboarding_done = true;
            }
        }
        m
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if !model.id_input_focused {
            return None;
        }
        // Enter o Escape → cerrar la edición.
        if event.state == KeyState::Pressed {
            match &event.key {
                Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Escape) => {
                    return Some(Msg::BlurIdInput);
                }
                _ => {}
            }
        }
        Some(Msg::IdInputKey(event.clone()))
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let shown = displayed_world(model);
        let stats = WorldStats::from_world(shown);

        let status = status_bar(model, &theme);
        // PsiMetrics es O(N²) por Moran — para N≈500 son ~250k operaciones
        // por frame a 11 Hz, perfectamente costeable y nos da las métricas
        // psicológicas en vivo sin un segundo bucle de cálculo.
        let psi_metrics = PsiMetrics::from_world(shown);
        let mut plan = build_plan_with_overrides(
            shown,
            &model.iso,
            &model.weights,
            &model.cfg,
            |i| lemming_color_for(model, i),
        );
        if model.show_trails && model.rewind_offset == 0 {
            overlay_trails(&mut plan, model);
        }
        let plan_cx = (plan.min_x + plan.max_x) * 0.5;
        let plan_cy = (plan.min_y + plan.max_y) * 0.5;
        let iso = model.iso;
        let canvas = canvas_pane(plan)
            .on_click_at(move |lx, ly, rw, rh| {
                // Mapeo inverso al que aplica canvas-llimphi para centrar la maqueta:
                //   plan_pos = local - rect/2 + plan_center
                let plan_x = lx - rw * 0.5 + plan_cx;
                let plan_y = ly - rh * 0.5 + plan_cy;
                let (wx, wy) = iso.unproject_floor(plan_x, plan_y);
                let max = (GRID as f32) - 1.0;
                if wx >= 0.0 && wx <= max && wy >= 0.0 && wy <= max {
                    Some(Msg::CanvasClick(wx, wy))
                } else {
                    None
                }
            })
            .draggable_at(move |phase, dx, dy, _lx0, _ly0| match phase {
                DragPhase::Move => {
                    // La inversa iso es lineal → unproject(dx, dy) = delta de mundo.
                    let (wdx, wdy) = iso.unproject_floor(dx, dy);
                    if wdx == 0.0 && wdy == 0.0 {
                        None
                    } else {
                        Some(Msg::CanvasDragMove(wdx, wdy))
                    }
                }
                DragPhase::End => None,
            });
        let side = side_panel(model, &stats, &psi_metrics, &theme);

        let body = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            min_size: Size {
                width: length(0.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![canvas, side]);

        let mut frame: Vec<View<Msg>> = vec![status];
        if !model.onboarding_done {
            frame.push(onboarding_bar(&theme));
        }
        frame.push(body);
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(frame)
    }
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Dominium>();
}
