//! Vista de la hoja imprimible: previsualización en pantalla y árbol de
//! `View` que se rasteriza a PNG por el módulo `print`.

use cosmos_render::{compose_wheel_with_hits, CompositionOpts, Palette};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};
use llimphi_widget_panel::{panel_signature_painter, PanelStyle};

use crate::model::{CosmosConfig, Model, Msg, MENU_BAR_H, STATUS_H, TAB_BAR_H};
use crate::view;

/// Lado del lienzo de la rueda en la hoja imprimible (px lógicos).
const PRINT_WHEEL: f32 = 528.0;
/// Ancho de la hoja imprimible (px lógicos).
const PRINT_SHEET_W: f32 = 600.0;
/// Alto de la hoja imprimible: proporción tamaño carta (8.5 × 11"), o sea
/// 11/8.5 ≈ 1.294 del ancho → 600 × 1.294 ≈ 776 px. Se usa como mínimo: la
/// hoja siempre llena un papel carta, y crece sólo si los aspectos no caben.
const PRINT_SHEET_H: f32 = 776.0;

/// La rueda natal estándar para la hoja: paleta clara sobre papel blanco,
/// sin zoom/paneo ni interactividad (es para imprimir). Caja fija de lado
/// `size`, centrada horizontalmente.
fn print_wheel(cfg: &CosmosConfig, render: &cosmos_render::RenderModel, size: f32) -> View<Msg> {
    let opts = CompositionOpts {
        size,
        rot_offset_deg: cfg.rot_offset_deg,
        include_bodies: true,
        palette: Palette::print(),
        draw_ascensional_cross: cfg.asc_cross,
        show_coord_labels: cfg.coord_labels,
        show_minor_aspects: cfg.minor_aspects,
        dial_3d: false,
        selected_body: None,
        detail: 1.0,
    };
    let (commands, _hits) = compose_wheel_with_hits(render, &opts);
    let canvas = cosmos_canvas_llimphi::canvas_view::<Msg>(
        commands,
        size,
        Some(Color::from_rgba8(255, 255, 255, 255)),
    );
    // Caja fija: el canvas mide percent(100%), necesita un rect definido.
    let boxed = View::new(Style {
        size: Size {
            width: length(size),
            height: length(size),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![canvas]);
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(size),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![boxed])
}

/// Contenido de la hoja imprimible (sin botón): cabecera de la carta +
/// rueda natal + tabla de aspectos, sobre papel blanco. Es EXACTAMENTE lo
/// que se rasteriza a PNG — el mismo árbol de `View`, la misma pintura —
/// así que la impresión tiene la fidelidad de la pantalla. Usa siempre el
/// tema «Print» (B/N) sin importar el tema de la app: el papel es blanco.
pub(crate) fn print_page_content(model: &Model) -> View<Msg> {
    print_page(&model.chart, &model.render, &model.cfg)
}

/// Arma el árbol de la hoja imprimible a partir de sus datos crudos (carta,
/// render, config). Separado de `print_page_content` para poder rasterizarlo
/// en un test sin un `Model` completo.
pub(crate) fn print_page(
    chart: &cosmos_model::Chart,
    render: &cosmos_render::RenderModel,
    cfg: &CosmosConfig,
) -> View<Msg> {
    let theme = Theme::print();
    // Maqueta apretada en proporción tamaño carta: la rueda arriba (ocupa
    // casi todo el ancho), los datos de nacimiento y los ángulos flotando
    // sobre sus esquinas superiores vacías (el círculo deja triángulos
    // libres), y la tabla de aspectos repartida en dos columnas abajo.
    let pad = 22.0_f32;
    let m_top = 14.0_f32;
    let m_side = 12.0_f32;
    // Datos de nacimiento: esquina superior izquierda, absoluto.
    let birth = View::new(Style {
        position: llimphi_ui::llimphi_layout::taffy::style::Position::Absolute,
        inset: Rect {
            left: length(m_side),
            top: length(m_top),
            right: auto(),
            bottom: auto(),
        },
        ..Default::default()
    })
    .children(vec![view::print_birth_block(chart, &theme)]);
    // Ángulos: esquina superior derecha, absoluto.
    let angles = View::new(Style {
        position: llimphi_ui::llimphi_layout::taffy::style::Position::Absolute,
        inset: Rect {
            right: length(m_side),
            top: length(m_top),
            left: auto(),
            bottom: auto(),
        },
        ..Default::default()
    })
    .children(vec![view::print_angles_block(render, &theme)]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(PRINT_SHEET_W),
            height: auto(),
        },
        // Mínimo = hoja carta completa; crece sólo si los aspectos rebasan.
        min_size: Size {
            width: length(PRINT_SHEET_W),
            height: length(PRINT_SHEET_H),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(pad),
            right: length(pad),
            top: length(pad),
            bottom: length(pad),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![
        // Rueda pegada arriba; los bloques de esquina la solapan.
        print_wheel(cfg, render, PRINT_WHEEL),
        view::section_label("Aspectos".to_string(), &theme),
        view::tile_aspectos_print(render, &theme),
        birth,
        angles,
    ])
}

/// Alto total de la hoja imprimible (papel carta como mínimo; crece si los
/// aspectos a dos columnas se pasan). Usado por el scroll de la vista.
pub(crate) fn print_sheet_h(render: &cosmos_render::RenderModel) -> f32 {
    // pad + rueda + gap + label(con margen) + gap + aspectos + pad.
    let content = 22.0 + PRINT_WHEEL + 6.0 + 22.0 + 6.0 + view::print_aspects_h(render) + 22.0;
    content.max(PRINT_SHEET_H)
}

/// Alto visible (viewport) de la previsualización de la hoja: el área
/// central menos el botón y los paddings de `print_view`.
pub(crate) fn print_viewport_h(model: &Model) -> f32 {
    (model.viewport.1 - MENU_BAR_H - TAB_BAR_H - STATUS_H - 60.0).max(80.0)
}

/// La vista en pantalla del modo «Hoja»: botón Imprimir arriba (fijo) + la
/// hoja (previsualización en papel) debajo, dentro de un área con scroll
/// vertical para hojas más altas que la ventana.
pub(crate) fn print_view(model: &Model, theme: &Theme) -> View<Msg> {
    let btn = View::new(Style {
        size: Size {
            width: length(190.0_f32),
            height: length(30.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(10.0_f32),
        },
        ..Default::default()
    })
    .radius(4.0)
    .fill(theme.bg_button)
    .hover_fill(theme.bg_button_hover)
    .text_aligned("Imprimir hoja…".to_string(), 13.0, theme.fg_text, Alignment::Center)
    .on_click(Msg::PrintSheet);

    // Hoja centrada horizontalmente dentro del ancho del viewport.
    let centered = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![print_page_content(model)]);

    let content = print_sheet_h(&model.render);
    let viewport = print_viewport_h(model);
    let offset = clamp_offset(model.print_scroll, content, viewport);
    let scroll = scroll_y(
        offset,
        content,
        viewport,
        centered,
        Msg::PrintScroll,
        &ScrollPalette::from_theme(theme),
    );
    let scroll_box = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(0.0_f32),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![scroll]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Start),
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(12.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .children(vec![btn, scroll_box])
}

/// Vista en mosaico de una celda de carta: etiqueta + su gráfica.
pub(crate) fn tile_cell_panel(
    label_view: View<Msg>,
    graphic: View<Msg>,
    theme: &Theme,
    tile_size: f32,
) -> View<Msg> {
    let ps = PanelStyle::from_theme(theme);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(tile_size + 16.0_f32),
            height: auto(),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .paint_with(panel_signature_painter(ps))
    .radius(ps.radius)
    .clip(true)
    .children(vec![label_view, graphic])
}
