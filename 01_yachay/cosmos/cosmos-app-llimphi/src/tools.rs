//! Panel de herramientas (derecha): rail vertical de categorías (tabs
//! estilo Photoshop) + acordeón de paneles colapsables dentro de la
//! categoría activa.
//!
//! Cada [`ToolPanel`] es una sección colapsable cuyo cuerpo reusa las
//! mismas funciones de tabla que ya existían (`view::tile_*`,
//! `astroview::view_*`). Aspectos (geocéntrico) y Aspectos topocéntrico
//! arrancan expandidos. El panel completo vive en una zona resizable
//! guardable; el usuario alterna categoría con el rail y abre/cierra cada
//! panel con su cabecera.

use cosmos_render::LayerKind;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

use llimphi_widget_panel::{panel_signature_painter, PanelStyle};
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};

use crate::astroview;
use crate::chrome;
use crate::glyphs::{self, Icon};
use crate::model::{Model, Msg, ToolCat, ToolPanel, MENU_BAR_H, STATUS_H, TOOLS_RAIL_W};
use crate::view;

/// Alto visible del contenedor de paneles (de bajo la barra de menú a
/// sobre la barra de estado).
pub(crate) fn tools_viewport_h(model: &Model) -> f32 {
    (model.viewport.1 - MENU_BAR_H - STATUS_H).max(60.0)
}

/// Alto total estimado del acordeón (cabecera de categoría + paneles).
/// Aproximado a partir del nº de filas de cada tabla — suficiente para
/// dimensionar la barra de scroll y acotar el offset.
pub(crate) fn tools_content_h(model: &Model) -> f32 {
    let mut h = 24.0 + 8.0; // cabecera de categoría + padding del acordeón
    for panel in model.tool_cat.panels() {
        h += HEAD_H + 6.0; // cabecera de la card + gap
        if model.panel_expanded(*panel) {
            h += panel_rows(*panel, model) as f32 * 20.0 + 22.0; // filas + padding
        }
    }
    h
}

/// Estimación del nº de filas (~20 px) del cuerpo de cada panel.
fn panel_rows(panel: ToolPanel, model: &Model) -> usize {
    let r = &model.render;
    let bodies = || {
        r.layers
            .iter()
            .filter(|l| l.module_id == "natal" && matches!(l.kind, LayerKind::Bodies))
            .flat_map(|l| l.glyphs.iter())
            .count()
    };
    let layer = |k: LayerKind| {
        r.layers
            .iter()
            .filter(|l| std::mem::discriminant(&l.kind) == std::mem::discriminant(&k))
            .flat_map(|l| l.glyphs.iter())
            .count()
    };
    match panel {
        ToolPanel::Carta => 10,
        ToolPanel::Aspectos | ToolPanel::AspectosTopo => 1 + r.aspect_summary.len().min(60),
        ToolPanel::Cuerpos => bodies().max(1),
        ToolPanel::Cualidades => 12,
        ToolPanel::Uraniano => r.uranian_groups.len().max(1),
        ToolPanel::BoxGraph => bodies().max(1),
        ToolPanel::Lotes => layer(LayerKind::Lots).max(1),
        ToolPanel::EstrellasFijas => layer(LayerKind::FixedStars).max(1),
        ToolPanel::PuntosMedios => layer(LayerKind::Midpoints).max(1),
        ToolPanel::Corpus => 14,
        ToolPanel::Cielo => 12,
        ToolPanel::OrtoOcaso => 12,
        ToolPanel::Sundial => 8,
        ToolPanel::Mareas => 10,
        ToolPanel::Eclipses => 10,
        ToolPanel::Efemerides => 14,
        ToolPanel::Configuracion => 22,
    }
}

/// Icono del rail vertical para cada categoría.
fn cat_icon(cat: ToolCat) -> Icon {
    match cat {
        ToolCat::Principal => Icon::Triangle,
        ToolCat::Analisis => Icon::Star,
        ToolCat::Astronomia => Icon::Moon,
        ToolCat::Sistema => Icon::Gear,
    }
}

/// El panel derecho completo: acordeón (crece) + rail de categorías
/// (fijo, pegado al borde). Pensado para vivir dentro de la zona
/// resizable de la derecha.
pub(crate) fn tools_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let accordion = accordion_view(model, theme);
    let rail = category_rail(model, theme);

    // Scroll vertical del acordeón: el contenido no se limita en alto; si
    // excede, el contenedor (no cada panel) hace scroll.
    let viewport = tools_viewport_h(model);
    let content = tools_content_h(model);
    let offset = clamp_offset(model.tools_scroll, content, viewport);
    let scroll = scroll_y(
        offset,
        content,
        viewport,
        accordion,
        Msg::ToolsScroll,
        &ScrollPalette::from_theme(theme),
    );
    let scroll_box = View::new(Style {
        flex_grow: 1.0,
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
    .children(vec![scroll]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![rail, scroll_box])
}

// =====================================================================
// Rail vertical de categorías (tabs estilo Photoshop)
// =====================================================================

/// Rail de categorías: tabs verticales **a la izquierda** del panel, sólo
/// del alto de sus dientes (no toda la vertical), pegado arriba para no
/// solapar con el lienzo. Cada diente lleva su icono; el activo va con un
/// borde de acento a la izquierda (estilo pestaña).
fn category_rail(model: &Model, theme: &Theme) -> View<Msg> {
    let mut btns: Vec<View<Msg>> = Vec::new();
    for cat in ToolCat::all() {
        let active = model.tool_cat == *cat;
        let fg = if active { theme.accent } else { theme.fg_muted };
        // Marca de acento a la izquierda del diente activo.
        let accent_bar = View::new(Style {
            size: Size {
                width: length(3.0_f32),
                height: length(40.0_f32),
            },
            flex_shrink: 0.0,
            ..Default::default()
        });
        let accent_bar = if active {
            accent_bar.fill(theme.accent).radius(2.0)
        } else {
            accent_bar
        };
        let icon_box = View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(0.0_f32),
                height: length(42.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .children(vec![glyphs::icon_view(cat_icon(*cat), 20.0, fg)]);
        let mut btn = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(42.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::SelectToolCat(*cat))
        .children(vec![accent_bar, icon_box]);
        if active {
            btn = btn.fill(theme.bg_selected);
        }
        btns.push(btn);
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        // Alto auto = sólo los dientes; alineado arriba.
        size: Size {
            width: length(TOOLS_RAIL_W),
            height: Dimension::auto(),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(4.0)
    .children(btns)
}

// =====================================================================
// Acordeón de paneles colapsables
// =====================================================================

fn accordion_view(model: &Model, theme: &Theme) -> View<Msg> {
    // Cabecera de la categoría activa (texto centrado vertical: nodo de
    // alto auto dentro de una fila centrada).
    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(10.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        ..Default::default()
    })
    .text_aligned(
        model.tool_cat.title().to_uppercase(),
        10.0,
        theme.fg_muted,
        Alignment::Start,
    )]);

    let mut kids: Vec<View<Msg>> = vec![header];
    for panel in model.tool_cat.panels() {
        kids.push(collapsible(*panel, model, theme));
    }

    // Alto natural (lo guía el contenido) — el scroll del contenedor lo
    // recorta. No `flex_grow` ni `clip` aquí.
    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_shrink: 0.0,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .children(kids)
}

const HEAD_H: f32 = 28.0;

/// Una sección colapsable como **card** con firma de panel (gradiente +
/// hairline) en la caja y una tira de cabecera con su propio gradiente.
/// El alto lo guía el contenido (auto), no el espacio disponible.
fn collapsible(panel: ToolPanel, model: &Model, theme: &Theme) -> View<Msg> {
    let expanded = model.panel_expanded(panel);
    let box_style = PanelStyle::from_theme(theme);
    // Cabecera: gradiente propio sobre bg_panel_alt; hairline sólo cuando
    // está expandida (refuerza la separación con el cuerpo).
    let mut head_style = PanelStyle::from_theme(theme);
    head_style.bg_base = theme.bg_panel_alt;
    head_style.radius = 0.0;
    head_style.hairline_alpha = if expanded { 0.30 } else { 0.0 };

    let chevron_box = View::new(Style {
        size: Size {
            width: length(18.0_f32),
            height: length(HEAD_H),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![glyphs::icon_view(
        if expanded { Icon::ChevronDown } else { Icon::ChevronRight },
        12.0,
        theme.fg_muted,
    )]);

    // Título: alto auto → centrado vertical por el align_items de la fila.
    let title = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: Dimension::auto(),
        },
        ..Default::default()
    })
    .text_aligned(panel.title().to_string(), 12.0, theme.fg_text, Alignment::Start);

    let head = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(HEAD_H),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(6.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .paint_with(panel_signature_painter(head_style))
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::ToggleToolPanel(panel))
    .children(vec![chevron_box, title]);

    let mut kids = vec![head];
    if expanded {
        kids.push(
            View::new(Style {
                flex_direction: FlexDirection::Column,
                flex_shrink: 0.0,
                size: Size {
                    width: percent(1.0_f32),
                    height: Dimension::auto(),
                },
                min_size: Size {
                    width: length(0.0_f32),
                    height: length(0.0_f32),
                },
                ..Default::default()
            })
            .children(vec![body_for(panel, model, theme)]),
        );
    }

    // Card: gradiente de caja + esquinas redondeadas + clip.
    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_shrink: 0.0,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .paint_with(panel_signature_painter(box_style))
    .radius(box_style.radius)
    .clip(true)
    .children(kids)
}

/// Cuerpo de cada panel — reusa las tablas existentes.
fn body_for(panel: ToolPanel, model: &Model, theme: &Theme) -> View<Msg> {
    let r = &model.render;
    match panel {
        ToolPanel::Carta => view::tile_carta(model, theme),
        ToolPanel::Aspectos | ToolPanel::AspectosTopo => view::tile_aspectos(r, theme),
        ToolPanel::Cuerpos => view::tile_cuerpos(r, theme),
        ToolPanel::Cualidades => view::tile_cualidades(r, theme),
        ToolPanel::Uraniano => view::tile_uraniano(&r.uranian_groups, theme),
        ToolPanel::BoxGraph => view::tile_box_graph(r, theme),
        ToolPanel::Lotes => view::tile_layer_glyphs(
            r,
            LayerKind::Lots,
            "lots",
            "Activá la capa «Lotes» (menú Capas).",
            theme,
        ),
        ToolPanel::EstrellasFijas => view::tile_layer_glyphs(
            r,
            LayerKind::FixedStars,
            "fixed_stars",
            "Activá la capa «Estrellas fijas» (menú Capas).",
            theme,
        ),
        ToolPanel::PuntosMedios => view::tile_layer_glyphs(
            r,
            LayerKind::Midpoints,
            "midpoints",
            "Activá la capa «Puntos medios» (menú Capas).",
            theme,
        ),
        ToolPanel::Corpus => view::tile_corpus(r, &model.corpus, theme),
        // Paneles astronómicos: si `astro` aún se calcula en el worker,
        // pintamos "calculando…" en vez de bloquear el hilo de UI.
        ToolPanel::Cielo => match &model.astro {
            Some(a) => astroview::view_cielo(a, theme),
            None => astroview::calculando(theme),
        },
        ToolPanel::OrtoOcaso => match &model.astro {
            Some(a) => astroview::view_ortoocaso(a, theme),
            None => astroview::calculando(theme),
        },
        ToolPanel::Sundial => match &model.astro {
            Some(a) => astroview::view_sundial(a, theme),
            None => astroview::calculando(theme),
        },
        ToolPanel::Mareas => match &model.astro {
            Some(a) => astroview::view_mareas(a, theme),
            None => astroview::calculando(theme),
        },
        ToolPanel::Eclipses => match &model.astro {
            Some(a) => astroview::view_eclipses(a, theme),
            None => astroview::calculando(theme),
        },
        ToolPanel::Efemerides => match &model.astro {
            Some(a) => astroview::view_efemerides(a, theme),
            None => astroview::calculando(theme),
        },
        ToolPanel::Configuracion => chrome::config_view(model, theme),
    }
}
