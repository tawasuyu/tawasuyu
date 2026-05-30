//! Tile **Atestaciones**: lista verificada del grafo; seleccionar una fija
//! su claim como objeto de evaluación para el tile Política.

use llimphi_theme::Theme;
use llimphi_ui::View;
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};

use crate::model::{Model, Msg};

pub(crate) fn atestaciones_view(model: &Model, theme: &Theme) -> View<Msg> {
    let list_palette = ListPalette::from_theme(theme);

    let rows: Vec<ListRow<Msg>> = model
        .graph
        .attestations()
        .iter()
        .enumerate()
        .map(|(idx, att)| {
            let mark = if att.is_self_attested() {
                "[self]"
            } else {
                "      "
            };
            let attester_name = model
                .graph
                .identity(att.attester)
                .map(|i| i.display_name.as_str())
                .unwrap_or("?");
            ListRow {
                label: format!(
                    "{mark}  {att}  ←  {attester} · {pred} = {val}",
                    att = att.attester,
                    attester = attester_name,
                    pred = att.claim.predicate,
                    val = att.claim.value,
                ),
                selected: model.selected_attestation == Some(idx),
                on_click: Msg::SeleccionarAtestacion(idx),
            }
        })
        .collect();

    let total = rows.len();
    let caption = format!("{total} atestaciones verificadas · seleccioná una para evaluar política");

    list_view(ListSpec {
        rows,
        total,
        caption: Some(caption),
        truncated_hint: None,
        row_height: 22.0,
        palette: list_palette,
    })
}
