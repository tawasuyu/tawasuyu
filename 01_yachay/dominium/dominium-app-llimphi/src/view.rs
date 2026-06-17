//! Todas las vistas de la app: status bar, onboarding, canvas pane, panel
//! lateral con sus cuatro tabs y los widgets de fila/slider reutilizados.

use dominium_core::{Epoch, PsiMetrics, Trigger, WorldStats};
use dominium_render_plan::{Color, RenderMode};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, View};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette};

use crate::consts::{GRID, SIDE_WIDTH};
use crate::model::{Layer, Model, Msg, PanelTab, ParamSlot, ZSlot};
use crate::packs::scenario_packs;
use crate::sim::CLUSTER_COLORS;

/// Nombre humano de la acción atómica `0..5`.
fn action_name(b: u8) -> &'static str {
    match b {
        0 => "Mover",
        1 => "Extraer",
        2 => "Sincronizar",
        3 => "Intercambiar",
        4 => "Replicar",
        5 => "Degradar",
        _ => "?",
    }
}

/// Descripción del trigger para mostrar en el panel.
fn trigger_label(t: Trigger) -> String {
    match t {
        Trigger::Always => "Always".to_string(),
        Trigger::EnergiaBajo(v) => format!("EnergíaBajo({v:.0})"),
        Trigger::EdadSobre(v) => format!("EdadSobre({v})"),
    }
}

/// Banda informativa que cubre el ancho de la app y explica las tres
/// gestures básicas del canvas. Se muestra hasta que el usuario haga el
/// primer click (que también es la gesture más obvia). Tiene una X a la
/// derecha para cerrarla manualmente sin tocar el canvas.
pub(crate) fn onboarding_bar(theme: &Theme) -> View<Msg> {
    let hint_text = "Click vacío → crea concepto · Click sobre uno → selecciona · Drag → mover · Tabs arriba a la derecha";
    let label = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(hint_text, 11.5, theme.accent, Alignment::Start);
    let close_btn = View::new(Style {
        size: Size {
            width: length(28.0_f32),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![llimphi_widget_button::button_view::<Msg>(
        "✕",
        &ButtonPalette::from_theme(theme),
        Msg::DismissOnboarding,
    )]);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(14.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![label, close_btn])
}

pub(crate) fn status_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let estado = rimay_localize::t(if model.sim.running {
        "dominium-status-running"
    } else {
        "dominium-status-paused"
    });
    // Texto principal: tamaño · población · epoch · tick. El usuario lo
    // ve siempre, sin importar el tab del panel.
    let line = format!(
        "{}×{}  ·  pob {}  ·  epoch {}  ·  tick {}",
        GRID,
        GRID,
        model.sim.world.lemmings.len(),
        model.sim.epoch,
        model.sim.tick,
    );
    let label_view = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(line, 12.0, theme.fg_text, Alignment::Start);
    let estado_view = View::new(Style {
        size: Size {
            width: length(120.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(estado, 12.0, theme.accent, Alignment::End);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![label_view, estado_view])
}

pub(crate) fn canvas_pane(
    plan: std::sync::Arc<dominium_render_plan::RenderPlan>,
) -> View<Msg> {
    let canvas_bg = llimphi_ui::llimphi_raster::peniko::Color::from_rgba8(11, 13, 18, 255);
    let canvas = dominium_canvas_llimphi::canvas_view_arc::<Msg>(plan, Some(canvas_bg));
    View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .clip(true)
    .children(vec![canvas])
}

pub(crate) fn side_panel(
    model: &Model,
    stats: &WorldStats,
    psi_metrics: &PsiMetrics,
    theme: &Theme,
) -> View<Msg> {
    let btn_palette = ButtonPalette::from_theme(theme);
    let mut slider_palette = SliderPalette::from_theme(theme);
    // Comprimimos los slots para que entren en el sidebar de 240 px.
    slider_palette.label_width = 56.0;
    slider_palette.track_width = 90.0;
    slider_palette.value_width = 44.0;

    let header = label_view(&rimay_localize::t("dominium-header-sim"), 11.0, theme.fg_muted);

    let play_label = rimay_localize::t(if model.sim.running {
        "dominium-btn-pause"
    } else {
        "dominium-btn-resume"
    });
    let play_btn = sized_button(&play_label, &btn_palette, Msg::TogglePlay);
    let reset_btn = sized_button(
        &rimay_localize::t("dominium-btn-reseed"),
        &btn_palette,
        Msg::Reseed,
    );

    // --- Tab bar: 4 pestañas chiquitas en fila ---
    let tab_bar = tab_bar_view(model, &btn_palette, theme);

    // Header siempre visible: play/pause + reseed (los controles más usados,
    // independientes del tab).
    let mut children: Vec<View<Msg>> = vec![
        header,
        tab_bar,
        play_btn,
        reset_btn,
        separator(theme),
    ];

    // Contenido específico del tab actual.
    match model.panel_tab {
        PanelTab::Mundo => append_mundo_tab(&mut children, model, stats, theme, &btn_palette, &slider_palette),
        PanelTab::Conceptos => append_conceptos_tab(&mut children, model, theme, &btn_palette, &slider_palette),
        PanelTab::Psique => append_psique_tab(&mut children, model, stats, psi_metrics, theme, &btn_palette, &slider_palette),
        PanelTab::Vista => append_vista_tab(&mut children, model, theme, &btn_palette, &slider_palette),
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(SIDE_WIDTH),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        gap: Size {
            width: length(0.0_f32),
            height: length(10.0_f32),
        },
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(14.0_f32),
            bottom: length(14.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(children)
}

/// Línea horizontal de 1 px usada como separator entre secciones del panel.
fn separator(theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.border)
}

/// Barra horizontal con un botón por cada `PanelTab`. El tab activo se
/// resalta cambiando el `accent` del label (botón) — la palette de Llimphi
/// no expone "tab pill", así que usamos la convención de marcar el activo
/// con `▸`.
fn tab_bar_view(model: &Model, btn_palette: &ButtonPalette, _theme: &Theme) -> View<Msg> {
    let buttons: Vec<View<Msg>> = PanelTab::all()
        .into_iter()
        .map(|tab| {
            let active = tab == model.panel_tab;
            let label = if active {
                format!("▸ {}", tab.label())
            } else {
                tab.label().to_string()
            };
            let mut bp = btn_palette.clone();
            if active {
                bp.bg = btn_palette.bg_hover;
            }
            View::new(Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(26.0_f32),
                },
                flex_grow: 1.0,
                flex_basis: length(0.0_f32),
                ..Default::default()
            })
            .children(vec![llimphi_widget_button::button_view::<Msg>(
                &label,
                &bp,
                Msg::SelectTab(tab),
            )])
        })
        .collect();
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(26.0_f32),
        },
        gap: Size {
            width: length(4.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(buttons)
}

/// Tab "Mundo" — estado macro + sliders de motor + scenario picker.
fn append_mundo_tab(
    children: &mut Vec<View<Msg>>,
    model: &Model,
    stats: &WorldStats,
    theme: &Theme,
    btn_palette: &ButtonPalette,
    slider_palette: &SliderPalette,
) {
    children.push(label_view(
        &rimay_localize::t("dominium-header-metricas"),
        11.0,
        theme.fg_muted,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-population"),
        &stats.n.to_string(),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-epoca"),
        Epoch::classify(stats).label(),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-materia"),
        &format!("{:.0}", stats.total_materia),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-oro"),
        &format!("{:.0}", stats.total_oro),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-energia"),
        &format!("{:.0}", stats.total_energia),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-gini-energia"),
        &format!("{:.3}", stats.gini_energia),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-edad-media"),
        &format!("{:.1}", stats.mean_edad),
        theme,
    ));
    children.push(stat_row(
        "season×",
        &format!("{:.2}", model.sim.params.season_factor(model.sim.world.tick_count)),
        theme,
    ));

    children.push(separator(theme));
    children.push(label_view("[ ACCIONES ACTUALES ]", 11.0, theme.fg_muted));
    let action_labels: [(&str, usize); 6] = [
        ("dominium-action-mover", 0),
        ("dominium-action-extraer", 1),
        ("dominium-action-sincronizar", 2),
        ("dominium-action-intercambiar", 3),
        ("dominium-action-replicar", 4),
        ("dominium-action-degradar", 5),
    ];
    for (key, ai) in action_labels {
        children.push(stat_row(
            &rimay_localize::t(key),
            &stats.action_counts[ai].to_string(),
            theme,
        ));
    }

    children.push(separator(theme));
    children.push(label_view("[ MOTOR ]", 11.0, theme.fg_muted));
    children.push(param_slider("climb", model.sim.params.climb_cost, ParamSlot::ClimbCost, slider_palette));
    children.push(param_slider("move", model.sim.params.move_cost, ParamSlot::MoveCost, slider_palette));
    children.push(param_slider("diffuse", model.sim.params.diffusion_rate, ParamSlot::DiffusionRate, slider_palette));
    children.push(param_slider("entropy", model.sim.params.entropy_rate, ParamSlot::EntropyRate, slider_palette));
    children.push(param_slider("season T", model.sim.params.season_period as f32, ParamSlot::SeasonPeriod, slider_palette));
    children.push(param_slider("season A", model.sim.params.season_amplitude, ParamSlot::SeasonAmplitude, slider_palette));

    children.push(separator(theme));
    children.push(label_view("[ ECONOMÍA ]", 11.0, theme.fg_muted));
    children.push(param_slider("extraer", model.sim.params.extract_rate, ParamSlot::ExtractRate, slider_palette));
    children.push(param_slider("trueque", model.sim.params.trade_amount, ParamSlot::TradeAmount, slider_palette));
    children.push(param_slider("regrowth", model.sim.params.regrowth_rate, ParamSlot::RegrowthRate, slider_palette));
    children.push(param_slider("carga", model.sim.params.carrying_capacity, ParamSlot::CarryingCapacity, slider_palette));
    children.push(param_slider("metabol", model.sim.params.metabolic_cost, ParamSlot::MetabolicCost, slider_palette));
    children.push(param_slider("replica", model.sim.params.replicate_threshold, ParamSlot::ReplicateThreshold, slider_palette));
    children.push(param_slider("abundan", model.sim.params.abundance_threshold, ParamSlot::AbundanceThreshold, slider_palette));

    children.push(separator(theme));
    children.push(label_view("[ CINÉTICA ]", 11.0, theme.fg_muted));
    children.push(param_slider("velocid", model.sim.params.move_speed, ParamSlot::MoveSpeed, slider_palette));
    children.push(param_slider("sync", model.sim.params.sync_rate, ParamSlot::SyncRate, slider_palette));
    children.push(param_slider("cicatriz", model.sim.params.degr_per_extract, ParamSlot::DegrPerExtract, slider_palette));
    children.push(param_slider("herencia", model.sim.params.child_energy_frac, ParamSlot::ChildEnergyFrac, slider_palette));
    children.push(param_slider("daño", model.sim.params.fight_damage, ParamSlot::FightDamage, slider_palette));
    children.push(param_slider("absorbe", model.sim.params.absorb_frac, ParamSlot::AbsorbFrac, slider_palette));
    children.push(param_slider("desespe", model.sim.params.desperation_threshold, ParamSlot::DesperationThreshold, slider_palette));
    children.push(param_slider("edad max", model.sim.params.max_edad as f32, ParamSlot::MaxEdad, slider_palette));

    children.push(separator(theme));
    children.push(label_view("[ SCENARIO ]", 11.0, theme.fg_muted));
    let packs = scenario_packs();
    let (current_id, _) = packs[model.scenario_idx];
    children.push(sized_button(
        &format!("pack: {} (▸ ciclar)", current_id),
        btn_palette,
        Msg::CycleScenario,
    ));
    children.push(sized_button(
        &rimay_localize::t_args("dominium-btn-load-named", &[("name", current_id.into())]),
        btn_palette,
        Msg::LoadScenario,
    ));
    children.push(separator(theme));
    children.push(label_view(&format!("grilla {GRID}×{GRID}"), 11.0, theme.fg_muted));
}

/// Tab "Conceptos" — lista de conceptos, crear/cargar/guardar/limpiar,
/// y el editor del Concepto seleccionado (radius, sprite, 4 mods, hack).
fn append_conceptos_tab(
    children: &mut Vec<View<Msg>>,
    model: &Model,
    theme: &Theme,
    btn_palette: &ButtonPalette,
    slider_palette: &SliderPalette,
) {
    children.push(label_view(
        &rimay_localize::t("dominium-header-conceptos"),
        11.0,
        theme.fg_muted,
    ));
    children.push(label_view(
        &rimay_localize::t_args(
            "dominium-active-count",
            &[("count", model.sim.world.conceptos.len().to_string().into())],
        ),
        12.0,
        theme.fg_text,
    ));

    // Hint contextual: si no hay conceptos, le decimos cómo crear uno.
    if model.sim.world.conceptos.items.is_empty() {
        children.push(label_view(
            "Click sobre el mapa para crear",
            11.0,
            theme.fg_muted,
        ));
    }

    for (i, c) in model.sim.world.conceptos.items.iter().enumerate() {
        children.push(concepto_row(i, &c.id, model.selected == Some(i), theme));
    }
    children.push(sized_button(
        &rimay_localize::t("dominium-btn-create-concept"),
        btn_palette,
        Msg::CrearConcepto,
    ));
    children.push(sized_button(
        &rimay_localize::t("dominium-btn-seed-pack"),
        btn_palette,
        Msg::SembrarConceptos,
    ));
    children.push(sized_button(
        &rimay_localize::t("dominium-btn-clear"),
        btn_palette,
        Msg::LimpiarConceptos,
    ));
    children.push(sized_button(
        &rimay_localize::t("dominium-btn-save"),
        btn_palette,
        Msg::GuardarPack,
    ));
    children.push(sized_button(
        &rimay_localize::t("dominium-btn-load-saved"),
        btn_palette,
        Msg::CargarPack,
    ));

    // Editor del seleccionado.
    let Some(i) = model.selected else { return };
    let Some(c) = model.sim.world.conceptos.items.get(i) else { return };
    children.push(separator(theme));
    children.push(label_view(
        &rimay_localize::t("dominium-header-editar"),
        11.0,
        theme.fg_muted,
    ));
    if model.id_input_focused {
        children.push(text_input_view(
            &model.id_input,
            &rimay_localize::t("dominium-slider-nombre"),
            true,
            &TextInputPalette::from_theme(theme),
            Msg::FocusIdInput,
        ));
    } else {
        children.push(sized_button(
            &format!("• {}  (✎ renombrar)", c.id),
            btn_palette,
            Msg::FocusIdInput,
        ));
    }
    children.push(slider_view(
        &rimay_localize::t("dominium-slider-radius"),
        c.radius,
        0.5,
        20.0,
        slider_palette,
        |phase, dv| match phase {
            DragPhase::Move => Some(Msg::EditRadius(dv)),
            DragPhase::End => None,
        },
    ));
    children.push(sized_button(
        &format!(
            "sprite: {} ({})",
            c.sprite_id,
            dominium_render_plan::sprite_name(c.sprite_id)
        ),
        btn_palette,
        Msg::CycleSprite,
    ));
    children.push(mod_slider(
        &rimay_localize::t("dominium-slider-materia"),
        c.mods.materia,
        Layer::Materia,
        slider_palette,
    ));
    children.push(mod_slider(
        &rimay_localize::t("dominium-slider-psique"),
        c.mods.psique,
        Layer::Psique,
        slider_palette,
    ));
    children.push(mod_slider(
        &rimay_localize::t("dominium-slider-poder"),
        c.mods.poder,
        Layer::Poder,
        slider_palette,
    ));
    children.push(mod_slider(
        &rimay_localize::t("dominium-slider-oro"),
        c.mods.oro,
        Layer::Oro,
        slider_palette,
    ));

    children.push(label_view(
        &rimay_localize::t("dominium-label-hack"),
        11.0,
        theme.fg_muted,
    ));
    match c.hack {
        None => {
            children.push(sized_button(
                "+ Agregar hack",
                btn_palette,
                Msg::HackToggle,
            ));
        }
        Some(h) => {
            children.push(sized_button(
                &format!("trigger: {}", trigger_label(h.trigger)),
                btn_palette,
                Msg::HackCycleTrigger,
            ));
            match h.trigger {
                Trigger::Always => {}
                Trigger::EnergiaBajo(v) => {
                    children.push(slider_view(
                        "umbral",
                        v,
                        0.0,
                        100.0,
                        slider_palette,
                        |phase, dv| match phase {
                            DragPhase::Move => Some(Msg::HackEditTriggerParam(dv)),
                            DragPhase::End => None,
                        },
                    ));
                }
                Trigger::EdadSobre(v) => {
                    children.push(slider_view(
                        "edad",
                        v as f32,
                        0.0,
                        1000.0,
                        slider_palette,
                        |phase, dv| match phase {
                            DragPhase::Move => Some(Msg::HackEditTriggerParam(dv)),
                            DragPhase::End => None,
                        },
                    ));
                }
            }
            children.push(sized_button(
                &format!("acción: {} ({})", h.forced_action, action_name(h.forced_action)),
                btn_palette,
                Msg::HackCycleAction,
            ));
            children.push(slider_view(
                "duración",
                h.duration as f32,
                1.0,
                500.0,
                slider_palette,
                |phase, dv| match phase {
                    DragPhase::Move => Some(Msg::HackEditDuration(dv)),
                    DragPhase::End => None,
                },
            ));
            children.push(sized_button("− Quitar hack", btn_palette, Msg::HackToggle));
        }
    }
    children.push(sized_button("🗑  Borrar", btn_palette, Msg::DeleteSelected));
    children.push(sized_button("◌  Deseleccionar", btn_palette, Msg::DeselectConcepto));
}

/// Tab "ψ" — sliders de psicología social + métricas ψ.
fn append_psique_tab(
    children: &mut Vec<View<Msg>>,
    model: &Model,
    stats: &WorldStats,
    psi_metrics: &PsiMetrics,
    theme: &Theme,
    btn_palette: &ButtonPalette,
    slider_palette: &SliderPalette,
) {
    children.push(label_view("[ DIVERSIDAD ψ ]", 11.0, theme.fg_muted));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-var-psi-orden"),
        &format!("{:.3}", stats.var_psi[0]),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-var-psi-miedo"),
        &format!("{:.3}", stats.var_psi[1]),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-var-psi-curiosidad"),
        &format!("{:.3}", stats.var_psi[2]),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-var-psi-corruptib"),
        &format!("{:.3}", stats.var_psi[3]),
        theme,
    ));

    children.push(separator(theme));
    children.push(label_view("[ CONTAGIO SOCIAL ]", 11.0, theme.fg_muted));
    children.push(param_slider(
        "psi mod",
        model.sim.params.psi_effect_modulation,
        ParamSlot::PsiModulation,
        slider_palette,
    ));
    children.push(param_slider(
        "radio soc",
        model.sim.params.social_radius,
        ParamSlot::SocialRadius,
        slider_palette,
    ));
    children.push(param_slider(
        "contagio",
        model.sim.params.contagion_rate,
        ParamSlot::ContagionRate,
        slider_palette,
    ));
    children.push(param_slider(
        "homofilia",
        model.sim.params.homophily_threshold,
        ParamSlot::HomophilyThreshold,
        slider_palette,
    ));
    let big5_label = if model.sim.params.big_five {
        "✓  Big Five: ON (5D)"
    } else {
        "○  Big Five: OFF (4D)"
    };
    children.push(sized_button(big5_label, btn_palette, Msg::ToggleBigFive));
    let policy_label = match model.sim.params.action_policy {
        dominium_core::ActionPolicy::Fixed => "○  Política: Fixed".to_string(),
        dominium_core::ActionPolicy::PsiArgmax => format!(
            "✓  Política: PsiArgmax (T={})",
            model.sim.params.policy_reeval_period
        ),
    };
    children.push(sized_button(&policy_label, btn_palette, Msg::CyclePsiPolicy));

    children.push(separator(theme));
    children.push(label_view("[ POLARIZACIÓN Esteban-Ray ]", 11.0, theme.fg_muted));
    let psi_labels = ["ORDEN", "MIEDO", "CURIO", "CORR"];
    for (i, lab) in psi_labels.iter().enumerate() {
        children.push(stat_row(
            &format!("polar {lab}"),
            &format!("{:.4}", psi_metrics.polarization[i]),
            theme,
        ));
    }
    children.push(label_view("[ Moran's I (autocorr.) ]", 11.0, theme.fg_muted));
    for (i, lab) in psi_labels.iter().enumerate() {
        children.push(stat_row(
            &format!("Moran {lab}"),
            &format!("{:+.3}", psi_metrics.moran_i[i]),
            theme,
        ));
    }
    if model.sim.params.big_five {
        children.push(stat_row(
            "polar EXTRA",
            &format!("{:.4}", psi_metrics.polarization_ext),
            theme,
        ));
        children.push(stat_row(
            "Moran EXTRA",
            &format!("{:+.3}", psi_metrics.moran_i_ext),
            theme,
        ));
    }

    // Legend de clusters cuando el render está mostrando tribus.
    if matches!(model.cfg.render_mode, RenderMode::PsiCluster) {
        children.push(separator(theme));
        children.push(label_view("[ TRIBUS k-means ]", 11.0, theme.fg_muted));
        for (k, c) in CLUSTER_COLORS.iter().enumerate() {
            let n_in = model
                .sim
                .cluster_assignments
                .iter()
                .filter(|&&a| a as usize == k)
                .count();
            children.push(stat_row(
                &format!("cluster {k}  ({})", color_swatch(*c)),
                &n_in.to_string(),
                theme,
            ));
        }
    }
}

/// Tab "Vista" — render mode + trails + andina + ZWeights + rewind.
fn append_vista_tab(
    children: &mut Vec<View<Msg>>,
    model: &Model,
    theme: &Theme,
    btn_palette: &ButtonPalette,
    slider_palette: &SliderPalette,
) {
    children.push(label_view("[ MODO RENDER ]", 11.0, theme.fg_muted));
    let render_label = match model.cfg.render_mode {
        RenderMode::Composite => "Render: compuesto".to_string(),
        RenderMode::Heatmap(l) => format!("Render: heatmap {}", l.label()),
        RenderMode::PsiCluster => "Render: tribus ψ (k-means)".to_string(),
    };
    children.push(sized_button(&render_label, btn_palette, Msg::CycleRenderMode));
    let trails_label = if model.show_trails {
        "✓  Trayectorias: ON"
    } else {
        "○  Trayectorias: OFF"
    };
    children.push(sized_button(trails_label, btn_palette, Msg::ToggleTrails));
    let texture_label = if model.cfg.texture {
        "✓  Textura: ON"
    } else {
        "○  Textura: OFF"
    };
    children.push(sized_button(texture_label, btn_palette, Msg::ToggleTexture));
    let andina_label = if model.cfg.andina_layers > 0 {
        "✓  Estampa andina: ON"
    } else {
        "○  Estampa andina: OFF"
    };
    children.push(sized_button(andina_label, btn_palette, Msg::ToggleAndina));

    children.push(separator(theme));
    children.push(label_view("[ RELIEVE VISUAL ]", 11.0, theme.fg_muted));
    children.push(z_slider(
        &rimay_localize::t("dominium-slider-materia"),
        model.weights.materia,
        ZSlot::Materia,
        slider_palette,
    ));
    children.push(z_slider(
        &rimay_localize::t("dominium-slider-psique"),
        model.weights.psique,
        ZSlot::Psique,
        slider_palette,
    ));
    children.push(z_slider(
        &rimay_localize::t("dominium-slider-poder"),
        model.weights.poder,
        ZSlot::Poder,
        slider_palette,
    ));
    children.push(z_slider(
        &rimay_localize::t("dominium-slider-oro"),
        model.weights.oro,
        ZSlot::Oro,
        slider_palette,
    ));
    children.push(z_slider(
        "degrad.",
        model.weights.degradacion,
        ZSlot::Degradacion,
        slider_palette,
    ));
    let sync_label = if model.sync_relieve {
        "✓  Sync físico: ON"
    } else {
        "○  Sync físico: OFF"
    };
    children.push(sized_button(sync_label, btn_palette, Msg::ToggleSyncRelieve));

    children.push(separator(theme));
    children.push(label_view("[ REWIND ]", 11.0, theme.fg_muted));
    let max_rewind = model.sim.snapshots.len().saturating_sub(1).max(1);
    children.push(slider_view(
        "rewind",
        model.sim.rewind_offset as f32,
        0.0,
        max_rewind as f32,
        slider_palette,
        |phase, dv| match phase {
            DragPhase::Move => Some(Msg::RewindBy(dv)),
            DragPhase::End => None,
        },
    ));
    if model.sim.rewind_offset > 0 {
        children.push(sized_button(
            &format!("▶  Vivo (estabas {} atrás)", model.sim.rewind_offset),
            btn_palette,
            Msg::RewindHome,
        ));
    }
}

/// Glifo simple para indicar el color de un cluster en una fila de stat.
/// El texto es monoespaciado pero los colores van en el panel — usamos
/// emojis círculos para que el matching visual sea inmediato sin tocar el
/// renderer del label.
fn color_swatch(c: Color) -> &'static str {
    let r = c[0] > 0.6;
    let g = c[1] > 0.6;
    let b = c[2] > 0.6;
    match (r, g, b) {
        (true, false, true) => "magenta",
        (false, true, true) => "cian",
        (true, true, false) => "amarillo",
        _ => "·",
    }
}

fn label_view(text: &str, size_px: f32, color: llimphi_ui::llimphi_raster::peniko::Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(text.to_string(), size_px, color, Alignment::Start)
}

fn stat_row(label: &str, value: &str, theme: &Theme) -> View<Msg> {
    let label_v = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(label.to_string(), 12.0, theme.fg_muted, Alignment::Start);
    let value_v = View::new(Style {
        size: Size {
            width: length(90.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(value.to_string(), 12.0, theme.fg_text, Alignment::End);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![label_v, value_v])
}

fn sized_button(label: &str, palette: &ButtonPalette, msg: Msg) -> View<Msg> {
    let mut btn = button_view(label, palette, msg);
    btn.style.size = Size {
        width: percent(1.0_f32),
        height: length(30.0_f32),
    };
    btn
}

/// Fila clicable con el nombre de un Concepto. La fila seleccionada
/// queda resaltada con `bg_selected`; las demás reaccionan al hover.
fn concepto_row(i: usize, id: &str, selected: bool, theme: &Theme) -> View<Msg> {
    let bg = if selected { theme.bg_selected } else { theme.bg_panel };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(theme.bg_row_hover)
    .radius(3.0)
    .text_aligned(
        format!("·  {id}"),
        12.0,
        if selected { theme.accent } else { theme.fg_text },
        Alignment::Start,
    )
    .on_click(Msg::SelectConcepto(i))
}

/// Slider para una capa de `LayerMods`. Rango fijo `[-1, 1]` — encaja con
/// el patrón típico (emisión positiva, drenaje negativo).
fn mod_slider(label: &str, value: f32, layer: Layer, palette: &SliderPalette) -> View<Msg> {
    slider_view(
        label,
        value,
        -1.0,
        1.0,
        palette,
        move |phase, dv| match phase {
            DragPhase::Move => Some(Msg::EditMod(layer, dv)),
            DragPhase::End => None,
        },
    )
}

/// Slider para un slot de `SimParams`. El rango lo decide el slot.
fn param_slider(
    label: &str,
    value: f32,
    slot: ParamSlot,
    palette: &SliderPalette,
) -> View<Msg> {
    let (min, max) = slot.range();
    slider_view(
        label,
        value,
        min,
        max,
        palette,
        move |phase, dv| match phase {
            DragPhase::Move => Some(Msg::EditParam(slot, dv)),
            DragPhase::End => None,
        },
    )
}

/// Slider para un slot de `ZWeights` (relieve visual del render).
/// Rango simétrico [-2, 2]: negativo = la capa cava valles, positivo = eleva.
fn z_slider(
    label: &str,
    value: f32,
    slot: ZSlot,
    palette: &SliderPalette,
) -> View<Msg> {
    slider_view(
        label,
        value,
        -2.0,
        2.0,
        palette,
        move |phase, dv| match phase {
            DragPhase::Move => Some(Msg::EditZWeight(slot, dv)),
            DragPhase::End => None,
        },
    )
}
