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

use std::time::Duration;

use dominium_core::{BehaviorHack, Conceptos, PsiMetrics, SimParams, Trigger, WorldStats};
use dominium_iso::{IsoProjector, ZWeights};
use dominium_render_plan::{build_plan_with_overrides, PlanConfig, RenderLayer, RenderMode};
use dominium_sim::Sim;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    length, percent, Dimension, FlexDirection, Size, Style,
};
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_motion::{animate, motion, Tween};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_text_input::TextInputState;
use wawa_config_llimphi::theme_from_wawa;

use crate::consts::{GRID, KMEANS_REFRESH_TICKS, SNAPSHOT_RING_CAP, TICK_MS, TRAIL_CAP};
use crate::model::{Layer, Model, Msg, PanelTab, ParamSlot, ZSlot};
use crate::packs::{
    default_conceptos, load_user_escenario, save_user_escenario, scenario_packs,
};
use crate::sim::{
    lemming_color_for, mirror_zweights_to_relieve, overlay_trails, selected_mut, spawn_concepto_at,
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
        // Relieve por bioma, recalibrado para los valores nuevos:
        //   - mares  → z ≈ −15 (psique 200 × −0.075)
        //   - llanura → z ≈ +1.6 (materia 80 × 0.02)
        //   - colinas → z ≈ +6 (poder 15 × 0.4)
        //   - picos  → z ≈ +21 (degradacion 16 × 1.3 + el resto)
        let mut weights = ZWeights {
            materia: 0.02,
            psique: -0.075,
            poder: 0.40,
            oro: 0.0,
            degradacion: 1.30,
        };
        // Si el usuario guardó un escenario rico, su sintonía gana sobre los
        // defaults de arriba (los Conceptos ya entraron por `seed()` →
        // `load_user_pack`). Packs históricos sólo traen Conceptos: dejan
        // `params`/`weights` intactos. Ver `packs::Escenario`.
        let mut params = params;
        if let Some(esc) = load_user_escenario() {
            if let Some(p) = esc.params {
                params = p;
            }
            if let Some(w) = esc.weights {
                weights = w;
            }
        }
        // Sesión de simulación: dominio + reloj + historia. El seeder
        // recarga el pack del usuario en cada reseed/colapso (igual que antes).
        let needs_psi5 = params.big_five;
        let mut sim = Sim::new(
            seed(rng_seed),
            params,
            rng_seed,
            SNAPSHOT_RING_CAP,
            TRAIL_CAP,
            KMEANS_REFRESH_TICKS,
            true,
            Box::new(|s| seed(s)),
        );
        // El mundo recién sembrado nace Big Four (psi5 vacío); si el
        // escenario guardado pide Big Five, rellenamos la quinta columna
        // antes del primer tick.
        if needs_psi5 {
            sim.world.lemmings.ensure_psi5_len();
        }
        Model {
            sim,
            // Scale 3.0 para que la grilla 240×240 entre en pantalla. z_factor
            // 0.55 levanta el relieve a algo perceptible sin que los picos
            // exploten: mares ~−25 px, llanura plana, colinas ~+10 px,
            // picos ~+30 px. La versión 0.35 anterior daba total ~9 px → el
            // mapa parecía plano por completo.
            iso: IsoProjector::new(3.0, 0.55),
            weights,
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
            selected: None,
            sync_relieve: false,
            id_input: TextInputState::new(),
            id_input_focused: false,
            scenario_idx: 0,
            show_trails: false,
            theme,
            _wawa_watcher: wawa_watcher,
            panel_tab: PanelTab::Mundo,
            onboarding_done: false,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_menu: None,
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
            clipboard: llimphi_clipboard::SystemClipboard::new(),
        }
    }

    fn update(model: Model, msg: Msg, h: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                // Si el usuario está revisando el pasado, la sim queda
                // congelada para no acumular divergencia con el ring.
                if m.sim.running && m.sim.rewind_offset == 0 {
                    m.sim
                        .advance(matches!(m.cfg.render_mode, RenderMode::PsiCluster));
                }
            }
            Msg::TogglePlay => {
                m.sim.running = !m.sim.running;
            }
            Msg::Reseed => {
                m.sim.reseed();
            }
            Msg::LimpiarConceptos => {
                m.sim.world.conceptos.clear();
                // Romper los hack_locks vivos: sin Concepto que los sostenga,
                // los lemmings vuelven a la lógica normal.
                for lock in m.sim.world.lemmings.hack_lock.iter_mut() {
                    *lock = 0;
                }
                m.selected = None;
            }
            Msg::SembrarConceptos => {
                m.sim.world.conceptos = default_conceptos();
                m.selected = None;
            }
            Msg::SelectConcepto(i) => {
                if i < m.sim.world.conceptos.len() {
                    m.selected = Some(i);
                }
            }
            Msg::DeselectConcepto => m.selected = None,
            Msg::EditMod(layer, dv) => {
                if let Some(i) = m.selected {
                    if let Some(c) = m.sim.world.conceptos.items.get_mut(i) {
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
                    if let Some(c) = m.sim.world.conceptos.items.get_mut(i) {
                        c.radius = (c.radius + dv).clamp(0.5, 20.0);
                    }
                }
            }
            Msg::DeleteSelected => {
                if let Some(i) = m.selected.take() {
                    if i < m.sim.world.conceptos.len() {
                        m.sim.world.conceptos.remove(i);
                        for lock in m.sim.world.lemmings.hack_lock.iter_mut() {
                            *lock = 0;
                        }
                    }
                }
            }
            Msg::EditParam(slot, dv) => {
                let (lo, hi) = slot.range();
                match slot {
                    ParamSlot::ClimbCost => {
                        m.sim.params.climb_cost = (m.sim.params.climb_cost + dv).clamp(lo, hi)
                    }
                    ParamSlot::DiffusionRate => {
                        m.sim.params.diffusion_rate =
                            (m.sim.params.diffusion_rate + dv).clamp(lo, hi)
                    }
                    ParamSlot::EntropyRate => {
                        m.sim.params.entropy_rate = (m.sim.params.entropy_rate + dv).clamp(lo, hi)
                    }
                    ParamSlot::MoveCost => {
                        m.sim.params.move_cost = (m.sim.params.move_cost + dv).clamp(lo, hi)
                    }
                    ParamSlot::SeasonPeriod => {
                        let v = (m.sim.params.season_period as f32 + dv).clamp(lo, hi);
                        m.sim.params.season_period = v as u32;
                    }
                    ParamSlot::SeasonAmplitude => {
                        m.sim.params.season_amplitude =
                            (m.sim.params.season_amplitude + dv).clamp(lo, hi)
                    }
                    ParamSlot::PsiModulation => {
                        m.sim.params.psi_effect_modulation =
                            (m.sim.params.psi_effect_modulation + dv).clamp(lo, hi)
                    }
                    ParamSlot::SocialRadius => {
                        m.sim.params.social_radius =
                            (m.sim.params.social_radius + dv).clamp(lo, hi)
                    }
                    ParamSlot::ContagionRate => {
                        m.sim.params.contagion_rate =
                            (m.sim.params.contagion_rate + dv).clamp(lo, hi)
                    }
                    ParamSlot::HomophilyThreshold => {
                        m.sim.params.homophily_threshold =
                            (m.sim.params.homophily_threshold + dv).clamp(lo, hi)
                    }
                    ParamSlot::ExtractRate => {
                        m.sim.params.extract_rate = (m.sim.params.extract_rate + dv).clamp(lo, hi)
                    }
                    ParamSlot::TradeAmount => {
                        m.sim.params.trade_amount = (m.sim.params.trade_amount + dv).clamp(lo, hi)
                    }
                    ParamSlot::RegrowthRate => {
                        m.sim.params.regrowth_rate = (m.sim.params.regrowth_rate + dv).clamp(lo, hi)
                    }
                    ParamSlot::CarryingCapacity => {
                        m.sim.params.carrying_capacity =
                            (m.sim.params.carrying_capacity + dv).clamp(lo, hi)
                    }
                    ParamSlot::MetabolicCost => {
                        m.sim.params.metabolic_cost =
                            (m.sim.params.metabolic_cost + dv).clamp(lo, hi)
                    }
                    ParamSlot::ReplicateThreshold => {
                        m.sim.params.replicate_threshold =
                            (m.sim.params.replicate_threshold + dv).clamp(lo, hi)
                    }
                    ParamSlot::AbundanceThreshold => {
                        m.sim.params.abundance_threshold =
                            (m.sim.params.abundance_threshold + dv).clamp(lo, hi)
                    }
                    ParamSlot::MoveSpeed => {
                        m.sim.params.move_speed = (m.sim.params.move_speed + dv).clamp(lo, hi)
                    }
                    ParamSlot::SyncRate => {
                        m.sim.params.sync_rate = (m.sim.params.sync_rate + dv).clamp(lo, hi)
                    }
                    ParamSlot::DegrPerExtract => {
                        m.sim.params.degr_per_extract =
                            (m.sim.params.degr_per_extract + dv).clamp(lo, hi)
                    }
                    ParamSlot::ChildEnergyFrac => {
                        m.sim.params.child_energy_frac =
                            (m.sim.params.child_energy_frac + dv).clamp(lo, hi)
                    }
                    ParamSlot::FightDamage => {
                        m.sim.params.fight_damage = (m.sim.params.fight_damage + dv).clamp(lo, hi)
                    }
                    ParamSlot::AbsorbFrac => {
                        m.sim.params.absorb_frac = (m.sim.params.absorb_frac + dv).clamp(lo, hi)
                    }
                    ParamSlot::DesperationThreshold => {
                        m.sim.params.desperation_threshold =
                            (m.sim.params.desperation_threshold + dv).clamp(lo, hi)
                    }
                    ParamSlot::MaxEdad => {
                        let v = (m.sim.params.max_edad as f32 + dv).clamp(lo, hi);
                        m.sim.params.max_edad = v as u32;
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
                    mirror_zweights_to_relieve(&m.weights, &mut m.sim.params.relieve);
                }
            }
            Msg::GuardarPack => {
                save_user_escenario(&m.sim.params, &m.weights, &m.sim.world.conceptos)
            }
            Msg::CargarPack => {
                if let Some(esc) = load_user_escenario() {
                    m.sim.world.conceptos = esc.conceptos;
                    // Sintonía del motor: sólo se aplica si el pack la trae
                    // (los packs históricos no, y entonces conservamos la
                    // vigente). Si entra en Big Five, rellenamos psi5 antes
                    // de que el tick consulte la quinta columna.
                    if let Some(params) = esc.params {
                        let needs_psi5 = params.big_five;
                        m.sim.params = params;
                        if needs_psi5 {
                            m.sim.world.lemmings.ensure_psi5_len();
                        }
                    }
                    if let Some(weights) = esc.weights {
                        m.weights = weights;
                        if m.sync_relieve {
                            mirror_zweights_to_relieve(&m.weights, &mut m.sim.params.relieve);
                        }
                    }
                    for lock in m.sim.world.lemmings.hack_lock.iter_mut() {
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
                for (i, c) in m.sim.world.conceptos.items.iter().enumerate() {
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
                    mirror_zweights_to_relieve(&m.weights, &mut m.sim.params.relieve);
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
                if let Some(c) = m.selected.and_then(|i| m.sim.world.conceptos.items.get(i)) {
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
                    m.sim.world.conceptos = cs;
                    for lock in m.sim.world.lemmings.hack_lock.iter_mut() {
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
                    m.sim.refresh_clusters();
                }
            }
            Msg::ToggleTrails => {
                m.show_trails = !m.show_trails;
            }
            Msg::ToggleTexture => {
                m.cfg.texture = !m.cfg.texture;
            }
            Msg::RewindBy(dv) => {
                let cap = m.sim.snapshots.len().saturating_sub(1);
                let cur = m.sim.rewind_offset as f32;
                let next = (cur + dv).clamp(0.0, cap as f32);
                m.sim.rewind_offset = next as usize;
            }
            Msg::RewindHome => {
                m.sim.rewind_offset = 0;
            }
            Msg::ToggleBigFive => {
                m.sim.params.big_five = !m.sim.params.big_five;
                if m.sim.params.big_five {
                    // Saves Big Four que entraron sin columna psi5 hay que
                    // rellenarlos antes de que el motor consulte
                    // `lemmings.psi5[i]`.
                    m.sim.world.lemmings.ensure_psi5_len();
                }
            }
            Msg::CyclePsiPolicy => {
                m.sim.params.action_policy = match m.sim.params.action_policy {
                    dominium_core::ActionPolicy::Fixed => {
                        if m.sim.params.policy_reeval_period == 0 {
                            m.sim.params.policy_reeval_period = 20;
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
            Msg::MenuOpen(idx) => {
                m.menu_open = idx;
                m.menu_active = usize::MAX;
                // Abrir un menú principal cierra el contextual de edición.
                m.edit_menu = None;
                // Animación de aparición/swap: cada vez que se abre (o se
                // cambia de) menú, el dropdown se funde+desliza de nuevo.
                if idx.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(h, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuCommand(cmd) => {
                m.menu_open = None;
                return handle_menu_command(m, cmd, h);
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    m.menu_active = menubar_nav(&menu, mi, m.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    if let Some(cmd) = menubar_command_at(&menu, mi, m.menu_active) {
                        m.menu_open = None;
                        return handle_menu_command(m, cmd, h);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::EditNav(dir) => {
                let flags =
                    EditFlags::from_editor(m.id_input.editor(), m.id_input.is_masked());
                m.edit_active = editmenu::edit_menu_step(flags, m.edit_active, dir);
            }
            Msg::EditActivate => {
                let flags =
                    EditFlags::from_editor(m.id_input.editor(), m.id_input.is_masked());
                if let Some(action) = editmenu::edit_menu_action_at(flags, m.edit_active) {
                    m.edit_menu = None;
                    apply_edit_menu_action(&mut m, action);
                }
            }
            Msg::EditMenuOpen(x, y) => {
                // Sólo tiene sentido si hay un campo de texto focuseado;
                // si no, abrirlo igual sobre un editor vacío es inocuo
                // (todo aparece en gris), pero preferimos no molestar.
                if m.id_input_focused {
                    m.edit_menu = Some((x, y));
                    m.edit_active = usize::MAX;
                    m.menu_open = None;
                    m.edit_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(h, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::EditMenuAction(action) => {
                m.edit_menu = None;
                apply_edit_menu_action(&mut m, action);
            }
            Msg::CloseMenus => {
                m.menu_open = None;
                m.menu_active = usize::MAX;
                m.edit_menu = None;
                m.edit_active = usize::MAX;
            }
        }
        m
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state == KeyState::Pressed {
            // Menú principal abierto: flechas navegan, ←/→ cambian de menú
            // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc
            // cierra. Tiene prioridad sobre todo lo demás.
            if let Some(mi) = model.menu_open {
                let n = app_menu(model).menus.len().max(1);
                return match &event.key {
                    Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                    Key::Named(NamedKey::ArrowLeft) => Some(Msg::MenuOpen(Some((mi + n - 1) % n))),
                    Key::Named(NamedKey::ArrowRight) => Some(Msg::MenuOpen(Some((mi + 1) % n))),
                    Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                    Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                    Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                    _ => None,
                };
            }
            // Menú de edición abierto: ↑/↓ navegan, Enter ejecuta, Esc cierra.
            if model.edit_menu.is_some() {
                return match &event.key {
                    Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                    Key::Named(NamedKey::ArrowDown) => Some(Msg::EditNav(1)),
                    Key::Named(NamedKey::ArrowUp) => Some(Msg::EditNav(-1)),
                    Key::Named(NamedKey::Enter) => Some(Msg::EditActivate),
                    _ => None,
                };
            }
        }
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
        let shown = model.sim.displayed_world();
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
        if model.show_trails && model.sim.rewind_offset == 0 {
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

        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));

        let mut frame: Vec<View<Msg>> = vec![menubar, status];
        if !model.onboarding_done {
            frame.push(onboarding_bar(&theme));
        }
        frame.push(body);
        // El right-click se engancha en la raíz (origen 0,0 → las coords
        // locales que llegan al handler ya son de ventana) y abre el menú
        // de edición sobre el campo focuseado. El canvas tiene su propio
        // click/drag, pero no captura right-click, así que la raíz es el
        // catch-all.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .on_right_click_at(|x, y, _w, _h| Some(Msg::EditMenuOpen(x, y)))
        .children(frame)
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        let theme = model.theme;
        // El menú de edición tiene prioridad si está abierto.
        if let Some((x, y)) = model.edit_menu {
            let flags = EditFlags::from_editor(model.id_input.editor(), model.id_input.is_masked());
            let (w, hgt) = Self::initial_size();
            let mut spec = editmenu::edit_context_menu(
                (x, y),
                (w as f32, hgt as f32),
                &theme,
                flags,
                Msg::EditMenuAction,
                Msg::CloseMenus,
            );
            spec.active = model.edit_active;
            return Some(context_menu_view_ex(
                spec,
                ContextMenuExtras {
                    appear: model.edit_anim.value(),
                    ..Default::default()
                },
            ));
        }
        // Si no, el dropdown del menú principal.
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model, &theme),
            model.menu_active,
            model.menu_anim.value(),
        )
    }
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(
    menu: &'a app_bus::AppMenu,
    model: &Model,
    theme: &'a Theme,
) -> MenuBarSpec<'a, Msg> {
    let (w, h) = Dominium::initial_size();
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: (w as f32, h as f32),
        height: MENU_H,
        on_open: std::sync::Arc::new(Msg::MenuOpen),
        on_command: std::sync::Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// Construye el menú principal reflejando el estado actual de la sim.
/// El submenú Editar opera sobre `id_input` (el campo de renombre) y se
/// pone en gris cuando no hay nada focuseado o no hay nada que hacer.
fn app_menu(model: &Model) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};

    // Estado del campo de texto focuseado para el submenú Editar.
    let focused = model.id_input_focused;
    let ed = model.id_input.editor();
    let has_sel = focused && ed.has_selection();
    let can_undo = focused && ed.can_undo();
    let can_redo = focused && ed.can_redo();
    let has_text = focused && !ed.is_empty();

    let mut undo = MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z");
    if !can_undo {
        undo = undo.disabled();
    }
    let mut redo = MenuItem::new("Rehacer", "edit.redo").shortcut("Ctrl+Y");
    if !can_redo {
        redo = redo.disabled();
    }
    let mut cut = MenuItem::new("Cortar", "edit.cut").shortcut("Ctrl+X").separated();
    let mut copy = MenuItem::new("Copiar", "edit.copy").shortcut("Ctrl+C");
    if !has_sel {
        cut = cut.disabled();
        copy = copy.disabled();
    }
    let mut paste = MenuItem::new("Pegar", "edit.paste").shortcut("Ctrl+V");
    if !focused {
        paste = paste.disabled();
    }
    let mut sel_all = MenuItem::new("Seleccionar todo", "edit.selectall")
        .shortcut("Ctrl+A")
        .separated();
    if !has_text {
        sel_all = sel_all.disabled();
    }

    // Editar conceptos: sólo con selección.
    let has_concepto = model.selected.is_some();
    let mut borrar = MenuItem::new("Borrar concepto", "concepto.delete");
    let mut renombrar = MenuItem::new("Renombrar concepto…", "concepto.rename");
    if !has_concepto {
        borrar = borrar.disabled();
        renombrar = renombrar.disabled();
    }

    let play_label = if model.sim.running { "Pausar" } else { "Reproducir" };
    let mut rewind = MenuItem::new("Volver al presente", "sim.rewindhome");
    if model.sim.rewind_offset == 0 {
        rewind = rewind.disabled();
    }

    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Cargar pack de usuario", "file.loadpack"))
                .item(MenuItem::new("Guardar pack de usuario", "file.savepack").separated())
                .item(MenuItem::new("Ciclar scenario", "file.cyclescenario"))
                .item(MenuItem::new("Cargar scenario", "file.loadscenario")),
        )
        .menu(
            Menu::new("Editar")
                .item(undo)
                .item(redo)
                .item(cut)
                .item(copy)
                .item(paste)
                .item(sel_all)
                .item(renombrar)
                .item(borrar),
        )
        .menu(
            Menu::new("Simulación")
                .item(MenuItem::new(play_label, "sim.toggleplay").shortcut("Espacio"))
                .item(MenuItem::new("Re-sembrar mundo", "sim.reseed").separated())
                .item(MenuItem::new("Sembrar conceptos", "sim.sembrar"))
                .item(MenuItem::new("Limpiar conceptos", "sim.limpiar"))
                .item(MenuItem::new("Crear concepto", "sim.crear").separated())
                .item(MenuItem::new("Big Five ψ", "sim.bigfive"))
                .item(MenuItem::new("Política de acción ψ", "sim.psipolicy").separated())
                .item(rewind),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Ciclar modo de render", "view.rendermode"))
                .item(MenuItem::new("Trayectorias", "view.trails"))
                .item(MenuItem::new("Textura procedural", "view.texture"))
                .item(MenuItem::new("Terrazas andinas", "view.andina").separated())
                .item(MenuItem::new("Sincronizar relieve físico", "view.syncrelieve")),
        )
        .menu(
            Menu::new("Ayuda")
                .item(MenuItem::new("Mostrar guía de uso", "help.onboarding")),
        )
}

/// Traduce el `command` del menú principal al `Msg` real y lo despacha.
fn handle_menu_command(model: Model, command: String, h: &Handle<Msg>) -> Model {
    let target = match command.as_str() {
        "file.loadpack" => Some(Msg::CargarPack),
        "file.savepack" => Some(Msg::GuardarPack),
        "file.cyclescenario" => Some(Msg::CycleScenario),
        "file.loadscenario" => Some(Msg::LoadScenario),
        "edit.undo" => Some(Msg::EditMenuAction(EditAction::Undo)),
        "edit.redo" => Some(Msg::EditMenuAction(EditAction::Redo)),
        "edit.cut" => Some(Msg::EditMenuAction(EditAction::Cut)),
        "edit.copy" => Some(Msg::EditMenuAction(EditAction::Copy)),
        "edit.paste" => Some(Msg::EditMenuAction(EditAction::Paste)),
        "edit.selectall" => Some(Msg::EditMenuAction(EditAction::SelectAll)),
        "concepto.rename" => Some(Msg::FocusIdInput),
        "concepto.delete" => Some(Msg::DeleteSelected),
        "sim.toggleplay" => Some(Msg::TogglePlay),
        "sim.reseed" => Some(Msg::Reseed),
        "sim.sembrar" => Some(Msg::SembrarConceptos),
        "sim.limpiar" => Some(Msg::LimpiarConceptos),
        "sim.crear" => Some(Msg::CrearConcepto),
        "sim.bigfive" => Some(Msg::ToggleBigFive),
        "sim.psipolicy" => Some(Msg::CyclePsiPolicy),
        "sim.rewindhome" => Some(Msg::RewindHome),
        "view.rendermode" => Some(Msg::CycleRenderMode),
        "view.trails" => Some(Msg::ToggleTrails),
        "view.texture" => Some(Msg::ToggleTexture),
        "view.andina" => Some(Msg::ToggleAndina),
        "view.syncrelieve" => Some(Msg::ToggleSyncRelieve),
        "help.onboarding" => Some(Msg::DismissOnboarding),
        _ => None,
    };
    match target {
        Some(msg) => Dominium::update(model, msg, h),
        None => model,
    }
}

/// Aplica una acción del menú de edición al editor del campo focuseado
/// (`id_input`), replicando el bookkeeping de `Msg::IdInputKey`: si el
/// texto cambió, propaga el nuevo id al Concepto seleccionado.
fn apply_edit_menu_action(m: &mut Model, action: EditAction) {
    if !m.id_input_focused {
        return;
    }
    let r = editmenu::apply(m.id_input.editor_mut(), action, &mut m.clipboard);
    if r.changed() {
        let new_id = m.id_input.text().to_string();
        if let Some(c) = selected_mut(m) {
            c.id = new_id;
        }
    }
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Dominium>();
}
