//! Chrome del shell: barra de menú principal, árbol de navegación,
//! tira de pestañas, barra de estado, menús contextuales (overlay) y el
//! dispatch del contenido central según la vista activa.
//!
//! Los menús (principal y contextual) comparten una representación común
//! [`MenuEntry`]/[`MenuCmd`]: `view_overlay` arma los `ContextMenuItem`
//! desde la lista y `main::update` resuelve el índice clickeado contra la
//! misma lista — una sola fuente de verdad para que no se desincronicen.

use std::sync::Arc;

use cosmos_canvas_llimphi::canvas_view_clickable;
use cosmos_render::{compose_wheel_with_hits, CompositionOpts, LayerKind, Palette};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, View};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_segmented::{segmented_view, SegmentedPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_switch::{switch_view, SwitchPalette};
use llimphi_widget_tabs::{tabs_view, TabsPalette, TabsSpec};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};

use crate::astroview;
use crate::model::{
    Model, Msg, NavGroup, OverlayKind, ViewKind, WheelOpt, HARMONICS, MENU_BAR_H, MENU_BTN_W,
    NAV_WIDTH, STATUS_H, TAB_BAR_H, VIEWPORT, WHEEL_SIZE,
};
use crate::model::MenuKind;
use crate::persist::list_cards;
use crate::view;

// =====================================================================
// Entradas de menú compartidas (principal + contextual)
// =====================================================================

#[derive(Debug, Clone, Copy)]
pub(crate) enum MenuCmd {
    Sep,
    Nueva,
    Guardar,
    Duplicar,
    Recargar,
    Eliminar,
    Open(ViewKind),
    CerrarTab,
    Overlay(OverlayKind),
    Harmonic(u32),
    Theme(bool),
    AcercaDe,
    Wheel(WheelOpt),
    Deselect,
}

pub(crate) struct MenuEntry {
    label: String,
    pub(crate) cmd: MenuCmd,
    separator: bool,
    destructive: bool,
    enabled: bool,
    shortcut: Option<&'static str>,
}

impl MenuEntry {
    fn act(label: &str, cmd: MenuCmd) -> Self {
        Self {
            label: label.to_string(),
            cmd,
            separator: false,
            destructive: false,
            enabled: true,
            shortcut: None,
        }
    }
    fn act_string(label: String, cmd: MenuCmd) -> Self {
        Self {
            label,
            cmd,
            separator: false,
            destructive: false,
            enabled: true,
            shortcut: None,
        }
    }
    fn sep() -> Self {
        Self {
            label: String::new(),
            cmd: MenuCmd::Sep,
            separator: true,
            destructive: false,
            enabled: true,
            shortcut: None,
        }
    }
    fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
    fn enabled(mut self, b: bool) -> Self {
        self.enabled = b;
        self
    }
    fn shortcut(mut self, s: &'static str) -> Self {
        self.shortcut = Some(s);
        self
    }
    fn to_item(&self) -> ContextMenuItem {
        if self.separator {
            return ContextMenuItem::separator();
        }
        let mut it = ContextMenuItem::action(self.label.clone());
        if let Some(s) = self.shortcut {
            it = it.with_shortcut(s);
        }
        if !self.enabled {
            it = it.disabled();
        }
        if self.destructive {
            it = it.destructive();
        }
        it
    }
}

fn check(label: &str, on: bool) -> String {
    if on {
        format!("✓ {label}")
    } else {
        format!("   {label}")
    }
}

/// Entradas de un menú principal. `main::update` reusa esta función para
/// resolver el índice clickeado.
pub(crate) fn menu_entries(kind: MenuKind, m: &Model) -> Vec<MenuEntry> {
    match kind {
        MenuKind::Archivo => vec![
            MenuEntry::act("Nueva carta (ejemplo)", MenuCmd::Nueva),
            MenuEntry::act("Guardar carta en biblioteca", MenuCmd::Guardar).shortcut("Ctrl+S"),
            MenuEntry::act("Duplicar carta actual", MenuCmd::Duplicar),
            MenuEntry::act("Recargar desde disco", MenuCmd::Recargar),
            MenuEntry::sep(),
            MenuEntry::act("Eliminar carta seleccionada", MenuCmd::Eliminar)
                .destructive()
                .enabled(m.selected_card.is_some()),
        ],
        // No hay campos de texto editables: la carta se edita en el JSON
        // de disco y se recarga por watcher. El menú «Editar» reúne las
        // acciones reales sobre la selección/carta cargada.
        MenuKind::Editar => vec![
            MenuEntry::act("Quitar selección del cuerpo", MenuCmd::Deselect)
                .enabled(m.selected_body.is_some()),
            MenuEntry::sep(),
            MenuEntry::act("Recargar carta desde disco", MenuCmd::Recargar),
            MenuEntry::act("Guardar carta en biblioteca", MenuCmd::Guardar).shortcut("Ctrl+S"),
            MenuEntry::act("Duplicar carta actual", MenuCmd::Duplicar),
            MenuEntry::sep(),
            MenuEntry::act("Eliminar carta seleccionada", MenuCmd::Eliminar)
                .destructive()
                .enabled(m.selected_card.is_some()),
        ],
        MenuKind::Vista => {
            let mut v = Vec::new();
            for vk in ViewKind::astrologia() {
                v.push(MenuEntry::act(vk.title(), MenuCmd::Open(*vk)));
            }
            v.push(MenuEntry::sep());
            for vk in ViewKind::astronomia() {
                v.push(MenuEntry::act(vk.title(), MenuCmd::Open(*vk)));
            }
            v.push(MenuEntry::sep());
            v.push(MenuEntry::act("Configuración", MenuCmd::Open(ViewKind::Configuracion)));
            v.push(MenuEntry::sep());
            // Tema (espeja el toggle de Configuración) — «Ver: módulos/tema».
            v.push(MenuEntry::act_string(check("Tema oscuro", m.cfg.theme_dark), MenuCmd::Theme(true)));
            v.push(MenuEntry::act_string(check("Tema claro", !m.cfg.theme_dark), MenuCmd::Theme(false)));
            v.push(MenuEntry::sep());
            v.push(MenuEntry::act("Cerrar pestaña actual", MenuCmd::CerrarTab).shortcut("Ctrl+W"));
            v
        }
        MenuKind::Capas => OverlayKind::all()
            .iter()
            .map(|k| {
                MenuEntry::act_string(check(k.nombre(), m.overlays.contains(k)), MenuCmd::Overlay(*k))
            })
            .collect(),
        MenuKind::Armonico => HARMONICS
            .iter()
            .map(|h| MenuEntry::act_string(check(&format!("H{h}"), m.harmonic == *h), MenuCmd::Harmonic(*h)))
            .collect(),
        MenuKind::Ayuda => vec![MenuEntry::act("Acerca de cosmos", MenuCmd::AcercaDe)],
    }
}

/// Entradas del menú contextual de la rueda.
pub(crate) fn ctx_entries(m: &Model) -> Vec<MenuEntry> {
    let mut v = Vec::new();
    if m.selected_body.is_some() {
        v.push(MenuEntry::act("Quitar selección", MenuCmd::Deselect));
        v.push(MenuEntry::sep());
    }
    v.push(MenuEntry::act_string(
        check("Aspectos menores", m.cfg.minor_aspects),
        MenuCmd::Wheel(WheelOpt::MinorAspects),
    ));
    v.push(MenuEntry::act_string(
        check("Etiquetas de coordenadas", m.cfg.coord_labels),
        MenuCmd::Wheel(WheelOpt::CoordLabels),
    ));
    v.push(MenuEntry::act_string(
        check("Dial 3D", m.cfg.dial_3d),
        MenuCmd::Wheel(WheelOpt::Dial3d),
    ));
    v.push(MenuEntry::act_string(
        check("Cruz ascensional", m.cfg.asc_cross),
        MenuCmd::Wheel(WheelOpt::AscCross),
    ));
    v.push(MenuEntry::sep());
    v.push(MenuEntry::act("Duplicar carta", MenuCmd::Duplicar));
    v
}

// =====================================================================
// Barra de menú principal
// =====================================================================

pub(crate) fn menu_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let mut kids: Vec<View<Msg>> = Vec::new();

    // Pill de marca.
    kids.push(
        View::new(Style {
            size: Size {
                width: length(68.0_f32),
                height: length(20.0_f32),
            },
            flex_shrink: 0.0,
            margin: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(5.0_f32),
                bottom: length(5.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(theme.accent)
        .radius(4.0)
        .text_aligned("cosmos".to_string(), 11.0, theme.bg_app, Alignment::Center),
    );

    for k in MenuKind::order() {
        let active = model.menu_open == Some(*k);
        let mut btn = View::new(Style {
            size: Size {
                width: length(MENU_BTN_W),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text_aligned(k.label().to_string(), 12.0, theme.fg_text, Alignment::Center)
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::OpenMenu(*k));
        if active {
            btn = btn.fill(theme.bg_selected);
        }
        kids.push(btn);
    }

    // Spacer + etiqueta de la carta a la derecha.
    kids.push(
        View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(0.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        }),
    );
    kids.push(
        View::new(Style {
            size: Size {
                width: length(260.0_f32),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            padding: Rect {
                left: length(0.0_f32),
                right: length(12.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(model.chart.label.clone(), 11.0, theme.fg_muted, Alignment::End),
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(MENU_BAR_H),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(kids)
}

// =====================================================================
// Árbol de navegación
// =====================================================================

fn group_row(label: String, expanded: bool, g: NavGroup) -> TreeRow<Msg> {
    TreeRow {
        label,
        depth: 0,
        has_children: true,
        expanded,
        selected: false,
        on_toggle: Msg::ToggleNavGroup(g),
        on_select: Msg::ToggleNavGroup(g),
    }
}

fn view_row(v: ViewKind, model: &Model) -> TreeRow<Msg> {
    TreeRow {
        label: v.title().to_string(),
        depth: 1,
        has_children: false,
        expanded: false,
        selected: model.active_view() == v,
        on_toggle: Msg::SelectView(v),
        on_select: Msg::SelectView(v),
    }
}

pub(crate) fn nav_tree(model: &Model, theme: &Theme) -> View<Msg> {
    let mut rows: Vec<TreeRow<Msg>> = Vec::new();

    let cards = list_cards();
    rows.push(group_row(
        format!("Cartas ({})", cards.len()),
        model.exp_cartas,
        NavGroup::Cartas,
    ));
    if model.exp_cartas {
        for name in &cards {
            rows.push(TreeRow {
                label: name.clone(),
                depth: 1,
                has_children: false,
                expanded: false,
                selected: model.selected_card.as_deref() == Some(name.as_str()),
                on_toggle: Msg::CargarCarta(name.clone()),
                on_select: Msg::CargarCarta(name.clone()),
            });
        }
    }

    rows.push(group_row(
        "Astrología".to_string(),
        model.exp_astrologia,
        NavGroup::Astrologia,
    ));
    if model.exp_astrologia {
        for v in ViewKind::astrologia() {
            rows.push(view_row(*v, model));
        }
    }

    rows.push(group_row(
        "Astronomía".to_string(),
        model.exp_astronomia,
        NavGroup::Astronomia,
    ));
    if model.exp_astronomia {
        for v in ViewKind::astronomia() {
            rows.push(view_row(*v, model));
        }
    }

    rows.push(group_row("Sistema".to_string(), model.exp_sistema, NavGroup::Sistema));
    if model.exp_sistema {
        rows.push(view_row(ViewKind::Configuracion, model));
    }

    let tree = tree_view(TreeSpec {
        rows,
        row_height: 22.0,
        indent_px: 14.0,
        palette: TreePalette::from_theme(theme),
    });

    View::new(Style {
        size: Size {
            width: length(NAV_WIDTH),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![tree])
}

// =====================================================================
// Pestañas + contenido
// =====================================================================

pub(crate) fn tab_area(model: &Model, theme: &Theme) -> View<Msg> {
    let labels: Vec<String> = model.tabs.iter().map(|v| v.title().to_string()).collect();
    tabs_view(TabsSpec {
        labels,
        active: model.active_tab,
        on_select: Msg::ActivateTab,
        content: content_for(model, theme),
        tab_height: TAB_BAR_H,
        palette: TabsPalette::from_theme(theme),
        tab_width: None,
    })
}

fn content_for(model: &Model, theme: &Theme) -> View<Msg> {
    match model.active_view() {
        ViewKind::Rueda => rueda_view(model, theme),
        ViewKind::Cuerpos => view::tile_cuerpos(&model.render, theme),
        ViewKind::Aspectos => view::tile_aspectos(&model.render, "natal", theme),
        ViewKind::BoxGraph => view::tile_box_graph(&model.render, theme),
        ViewKind::Cualidades => view::tile_cualidades(&model.render, theme),
        ViewKind::Uraniano => view::tile_uraniano(&model.render.uranian_groups, theme),
        ViewKind::Lotes => view::tile_layer_glyphs(
            &model.render,
            LayerKind::Lots,
            "lots",
            "Activá la capa «Lotes» (menú Capas) para calcular los lotes helenísticos.",
            theme,
        ),
        ViewKind::EstrellasFijas => view::tile_layer_glyphs(
            &model.render,
            LayerKind::FixedStars,
            "fixed_stars",
            "Activá la capa «Estrellas fijas» (menú Capas).",
            theme,
        ),
        ViewKind::PuntosMedios => view::tile_layer_glyphs(
            &model.render,
            LayerKind::Midpoints,
            "midpoints",
            "Activá la capa «Puntos medios» (menú Capas).",
            theme,
        ),
        ViewKind::Corpus => view::tile_corpus(&model.render, &model.corpus, theme),
        ViewKind::AstroCarto => crate::astrocarto::tile_astrocarto(&model.chart, &model.render, theme),
        ViewKind::Cielo => astroview::view_cielo(&model.astro, theme),
        ViewKind::OrtoOcaso => astroview::view_ortoocaso(&model.astro, theme),
        ViewKind::Sundial => astroview::view_sundial(&model.astro, theme),
        ViewKind::Mareas => astroview::view_mareas(&model.astro, theme),
        ViewKind::Eclipses => astroview::view_eclipses(&model.astro, theme),
        ViewKind::Efemerides => astroview::view_efemerides(&model.astro, theme),
        ViewKind::Configuracion => config_view(model, theme),
    }
}

fn rueda_view(model: &Model, theme: &Theme) -> View<Msg> {
    let opts = CompositionOpts {
        size: WHEEL_SIZE,
        rot_offset_deg: model.cfg.rot_offset_deg,
        include_bodies: true,
        palette: Palette::dark(),
        draw_ascensional_cross: model.cfg.asc_cross,
        show_coord_labels: model.cfg.coord_labels,
        show_minor_aspects: model.cfg.minor_aspects,
        dial_3d: model.cfg.dial_3d,
        selected_body: model.selected_body.clone(),
    };
    let (commands, hits) = compose_wheel_with_hits(&model.render, &opts);
    let canvas_bg = Color::from_rgba8(8, 10, 16, 255);
    let canvas = canvas_view_clickable::<Msg, _>(commands, WHEEL_SIZE, Some(canvas_bg), move |wx, wy| {
        let picked: Option<String> = hits.pick(wx, wy).map(str::to_string);
        Some(Msg::SelectBody(picked))
    })
    // Click derecho → menú contextual. Las coords son locales al nodo
    // del canvas; lo sumamos al origen del área central (aprox).
    .on_right_click_at(|lx, ly, _w, _h| {
        Some(Msg::OpenCanvasCtx(NAV_WIDTH + lx, MENU_BAR_H + TAB_BAR_H + ly))
    });

    let canvas_box = View::new(Style {
        size: Size {
            width: length(WHEEL_SIZE),
            height: length(WHEEL_SIZE),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![canvas]);

    let info = View::new(Style {
        flex_grow: 1.0,
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(0.0_f32),
            height: percent(1.0_f32),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![view::tile_carta(model, theme)]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![canvas_box, info])
}

// =====================================================================
// Vista de configuración
// =====================================================================

fn switch_row(label: &str, on: bool, msg: Msg, pal: &SwitchPalette, theme: &Theme) -> View<Msg> {
    let lbl = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(label.to_string(), 12.0, theme.fg_text, Alignment::Start);

    let sw = View::new(Style {
        size: Size {
            width: length(44.0_f32),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![switch_view(if on { 1.0 } else { 0.0 }, msg, pal)]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![lbl, sw])
}

fn config_view(model: &Model, theme: &Theme) -> View<Msg> {
    let seg_pal = SegmentedPalette::from_theme(theme);
    let sw_pal = SwitchPalette::from_theme(theme);
    let sl_pal = SliderPalette::from_theme(theme);

    let mut rows: Vec<View<Msg>> = Vec::new();

    rows.push(view::section_label("Tema".to_string(), theme));
    rows.push(segmented_view(
        &["Oscuro", "Claro"],
        if model.cfg.theme_dark { 0 } else { 1 },
        |i| Msg::SetThemeDark(i == 0),
        &seg_pal,
    ));

    rows.push(view::section_label("Armónico".to_string(), theme));
    let h_idx = HARMONICS.iter().position(|h| *h == model.harmonic).unwrap_or(0);
    rows.push(segmented_view(
        &["H1", "H4", "H5", "H7", "H9"],
        h_idx,
        |i| Msg::SetHarmonic(HARMONICS.get(i).copied().unwrap_or(1)),
        &seg_pal,
    ));

    rows.push(view::section_label("Rueda".to_string(), theme));
    rows.push(switch_row(
        "Aspectos menores",
        model.cfg.minor_aspects,
        Msg::ToggleWheelOpt(WheelOpt::MinorAspects),
        &sw_pal,
        theme,
    ));
    rows.push(switch_row(
        "Etiquetas de coordenadas",
        model.cfg.coord_labels,
        Msg::ToggleWheelOpt(WheelOpt::CoordLabels),
        &sw_pal,
        theme,
    ));
    rows.push(switch_row(
        "Dial 3D",
        model.cfg.dial_3d,
        Msg::ToggleWheelOpt(WheelOpt::Dial3d),
        &sw_pal,
        theme,
    ));
    rows.push(switch_row(
        "Cruz ascensional",
        model.cfg.asc_cross,
        Msg::ToggleWheelOpt(WheelOpt::AscCross),
        &sw_pal,
        theme,
    ));
    rows.push(slider_view(
        "Rotación",
        model.cfg.rot_offset_deg,
        0.0,
        360.0,
        &sl_pal,
        |phase, dv| match phase {
            DragPhase::Move => Some(Msg::SetRotOffset(dv)),
            DragPhase::End => None,
        },
    ));

    rows.push(view::section_label("Astronomía".to_string(), theme));
    rows.push(switch_row(
        "Usar instante actual (ahora)",
        model.cfg.use_now,
        Msg::SetUseNow(!model.cfg.use_now),
        &sw_pal,
        theme,
    ));
    rows.push(view::line(
        format!("instante: {}", model.astro.instant_iso),
        11.0,
        theme.fg_muted,
    ));
    rows.push(view::line(
        format!("lugar: {}", model.astro.place_label),
        11.0,
        theme.fg_muted,
    ));

    rows.push(view::section_label("Capas".to_string(), theme));
    for k in OverlayKind::all() {
        rows.push(switch_row(
            k.nombre(),
            model.overlays.contains(k),
            Msg::ToggleOverlay(*k),
            &sw_pal,
            theme,
        ));
    }

    view::tile_container(rows, theme)
}

// =====================================================================
// Barra de estado
// =====================================================================

pub(crate) fn status_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let txt = if let Some(err) = &model.error {
        format!("error: {err}")
    } else if let Some(note) = &model.status_note {
        note.clone()
    } else {
        format!(
            "{}  ·  {} ms  ·  {} capas  ·  {} aspectos  ·  {} overlays",
            model.active_view().title(),
            model.render.compute_ms,
            model.render.layers.len(),
            model.render.aspect_summary.len(),
            model.render.overlays.len(),
        )
    };
    let color = if model.error.is_some() {
        theme.fg_destructive
    } else {
        theme.fg_muted
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(STATUS_H),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(txt, 11.0, color, Alignment::Start)
}

// =====================================================================
// Overlay (menú principal desplegado o menú contextual)
// =====================================================================

pub(crate) fn overlay_view(model: &Model, theme: &Theme) -> Option<View<Msg>> {
    let pal = ContextMenuPalette::from_theme(theme);
    if let Some(kind) = model.menu_open {
        let entries = menu_entries(kind, model);
        let items: Vec<ContextMenuItem> = entries.iter().map(MenuEntry::to_item).collect();
        return Some(context_menu_view(ContextMenuSpec {
            anchor: (kind.anchor_x(), MENU_BAR_H),
            viewport: VIEWPORT,
            header: Some(kind.label().to_uppercase()),
            items,
            active: usize::MAX,
            on_pick: Arc::new(move |i| Msg::MenuPick(kind, i)),
            on_dismiss: Msg::CloseMenu,
            palette: pal,
        }));
    }
    if let Some(anchor) = model.ctx_open {
        let entries = ctx_entries(model);
        let items: Vec<ContextMenuItem> = entries.iter().map(MenuEntry::to_item).collect();
        return Some(context_menu_view(ContextMenuSpec {
            anchor,
            viewport: VIEWPORT,
            header: Some("RUEDA".to_string()),
            items,
            active: usize::MAX,
            on_pick: Arc::new(Msg::CtxPick),
            on_dismiss: Msg::CloseCtx,
            palette: pal,
        }));
    }
    None
}
