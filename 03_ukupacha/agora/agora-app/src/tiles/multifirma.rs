//! Tile **Multifirma**: compone una [`MultiSignature`] M-of-N sobre un mensaje
//! (típicamente una raíz canónica) con identidades propias, y exporta postcard.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems,
};
use llimphi_ui::View;
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette};

use crate::model::{FocusedInput, Model, Msg};
use crate::tiles::kind_str;
use crate::ui::{boton_frac, button_palette_primary, button_palette_secondary, column, grow, label_line, spacer};

pub(crate) fn multifirma_view(model: &Model, theme: &Theme) -> View<Msg> {
    let input_palette = TextInputPalette::from_theme(theme);
    let list_palette = ListPalette::from_theme(theme);
    let slider_palette = SliderPalette::from_theme(theme);

    let mensaje_input = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(32.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![text_input_view(
        &model.multi_message,
        "raíz canónica / hash de manifiesto / …",
        matches!(model.focused_input, FocusedInput::MultiMessage),
        &input_palette,
        Msg::Foco(FocusedInput::MultiMessage),
    )]);

    // Identidades propias con check ☑/☐. La fila NO se pinta como selected
    // (el check ya lo indica; selected lo reservamos para focused_subject).
    let mut mias: Vec<_> = model
        .seeds
        .keys()
        .copied()
        .filter_map(|id| model.graph.identity(id).map(|i| (id, i)))
        .collect();
    mias.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let rows: Vec<ListRow<Msg>> = mias
        .iter()
        .map(|(id, ident)| {
            let check = if model.multi_selected.contains(id) {
                "☑"
            } else {
                "☐"
            };
            ListRow {
                label: format!(
                    "{check}  {id}  {kind}  {name}",
                    kind = kind_str(ident.kind),
                    name = ident.display_name
                ),
                selected: false,
                on_click: Msg::ToggleMultiFirmante(*id),
            }
        })
        .collect();
    let firmantes_list = list_view(ListSpec {
        rows,
        total: mias.len(),
        caption: Some(format!(
            "{} identidades propias · {} elegidas",
            mias.len(),
            model.multi_selected.len()
        )),
        truncated_hint: None,
        row_height: 22.0,
        palette: list_palette,
    });

    let n_seleccionados = model.multi_selected.len().max(1);
    let umbral_slider = slider_view(
        "umbral M",
        model.multi_threshold as f32,
        1.0,
        n_seleccionados as f32,
        &slider_palette,
        |phase, dv| Some(Msg::SliderMultiUmbral(phase, dv)),
    );

    let firmar = boton_frac(
        "firmar las elegidas (Enter)",
        1.0,
        30.0,
        &button_palette_primary(theme),
        Msg::FirmarMulti,
    );

    let acciones_secundarias = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        flex_shrink: 0.0,
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![
        boton_frac(
            "exportar postcard hex →",
            0.5,
            30.0,
            &button_palette_secondary(theme),
            Msg::ExportarMulti,
        ),
        boton_frac(
            "limpiar",
            0.5,
            30.0,
            &button_palette_secondary(theme),
            Msg::LimpiarMulti,
        ),
    ]);

    let veredicto_block: View<Msg> = match &model.multi_current {
        None => label_line(
            "(sin multifirma vigente — elegí firmantes, escribí el mensaje y Enter)",
            11.0,
            theme.fg_muted,
        ),
        Some(multi) => {
            let mensaje_bytes = model.multi_message.text();
            let v = multi.verdict(mensaje_bytes.as_bytes());
            let pasa = v.firmantes_distintos >= model.multi_threshold;
            let color = if pasa { theme.accent } else { theme.fg_destructive };
            let mut bloque = column(vec![
                label_line(
                    &format!(
                        "verdict: {} válidas · {} distintas · umbral {}",
                        v.validas, v.firmantes_distintos, model.multi_threshold
                    ),
                    11.0,
                    color,
                ),
                label_line(
                    if pasa {
                        "ACEPTA (umbral alcanzado)"
                    } else {
                        "rechaza (faltan firmantes distintos)"
                    },
                    14.0,
                    color,
                ),
            ]);
            // Alto fijo: el `column` 100% absorbía el espacio flexible del
            // tile y aplastaba la lista de firmantes con multifirma vigente.
            bloque.style.size.height = length(56.0_f32);
            bloque.style.flex_shrink = 0.0;
            bloque
        }
    };

    column(vec![
        spacer(6.0),
        label_line("mensaje a multifirmar", 10.0, theme.fg_muted),
        mensaje_input,
        spacer(8.0),
        grow(firmantes_list),
        spacer(6.0),
        umbral_slider,
        spacer(6.0),
        firmar,
        spacer(4.0),
        acciones_secundarias,
        spacer(8.0),
        veredicto_block,
    ])
}
