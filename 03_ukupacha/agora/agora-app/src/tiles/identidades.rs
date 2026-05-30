//! Tile **Identidades**: lista del grafo, alta de identidad propia y
//! selección de sujeto / firmante activo.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::View;
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};

use crate::model::{Model, Msg};
use crate::tiles::kind_str;
use crate::ui::{
    boton_frac, button_palette_primary, button_palette_secondary, column, edge_padding, grow,
};

pub(crate) fn identidades_view(model: &Model, theme: &Theme) -> View<Msg> {
    let list_palette = ListPalette::from_theme(theme);

    // Orden estable: por id bytes (graph.identities() itera el HashMap interno).
    let mut idents: Vec<_> = model.graph.identities().collect();
    idents.sort_by(|a, b| a.id().as_bytes().cmp(b.id().as_bytes()));

    let rows: Vec<ListRow<Msg>> = idents
        .iter()
        .map(|ident| {
            let id = ident.id();
            let prefix = if model.is_mine(id) { "★ " } else { "  " };
            let active = Some(id) == model.active_signer;
            let mark_active = if active { " ← activa" } else { "" };
            ListRow {
                label: format!(
                    "{prefix}{id}  {kind}  {name}{mark_active}",
                    kind = kind_str(ident.kind),
                    name = ident.display_name
                ),
                selected: model.focused_subject == Some(id),
                on_click: Msg::FocoSujeto(id),
            }
        })
        .collect();

    let caption = format!(
        "{} identidades · {} mías · enfocada: {}",
        idents.len(),
        model.seeds.len(),
        model
            .focused_subject
            .map(|id| format!("{id}"))
            .unwrap_or_else(|| "—".into())
    );

    let list = list_view(ListSpec {
        rows,
        total: idents.len(),
        caption: Some(caption),
        truncated_hint: None,
        row_height: 22.0,
        palette: list_palette,
    });

    let mut footer_buttons: Vec<View<Msg>> = vec![boton_frac(
        "+ nueva identidad",
        0.5,
        30.0,
        &button_palette_primary(theme),
        Msg::NuevaIdentidad,
    )];

    // "actuar como" — sólo si la enfocada es mía y distinta de la activa.
    let can_act_as = model
        .focused_subject
        .filter(|id| model.is_mine(*id) && Some(*id) != model.active_signer);
    if let Some(id) = can_act_as {
        footer_buttons.push(boton_frac(
            "actuar como ★ enfocada",
            0.5,
            30.0,
            &button_palette_secondary(theme),
            Msg::ActuarComo(id),
        ));
    }

    let footer = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(38.0_f32),
        },
        flex_shrink: 0.0,
        padding: edge_padding(8.0, 4.0),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Start),
        ..Default::default()
    })
    .children(footer_buttons);

    column(vec![grow(list), footer])
}
