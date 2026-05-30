//! Tile **Política**: ajusta los ejes de la `TrustPolicy` del LECTOR y pinta
//! en vivo el veredicto sobre la atestación seleccionada, con desglose por eje.

use agora_graph::Corroboration;
use llimphi_theme::Theme;
use llimphi_ui::View;
use llimphi_widget_slider::{slider_view, SliderPalette};

use crate::model::{Model, Msg};
use crate::tiles::{format_duration, format_max_age, kind_str};
use crate::ui::{boton_frac, button_palette_secondary, column, empty, grow, label_line, spacer};

pub(crate) fn politica_view(model: &Model, theme: &Theme) -> View<Msg> {
    let slider_palette = SliderPalette::from_theme(theme);

    let slider = slider_view(
        "min terceros",
        model.policy.min_third_party as f32,
        0.0,
        5.0,
        &slider_palette,
        |phase, dv| Some(Msg::SliderMinThird(phase, dv)),
    );

    let toggle = boton_frac(
        format!(
            "accept_self: {}",
            if model.policy.accept_self { "sí" } else { "no" }
        ),
        1.0,
        30.0,
        &button_palette_secondary(theme),
        Msg::ToggleAcceptSelf,
    );

    let kind_label = match model.policy.min_attesters_of_kind {
        None => "kind: off".to_string(),
        Some((k, _)) => format!("kind: {}", kind_str(k)),
    };
    let kind_button = boton_frac(
        kind_label,
        1.0,
        30.0,
        &button_palette_secondary(theme),
        Msg::CycleKind,
    );

    // Slider del N sólo si el eje kind está activo; si no, un hint discreto
    // para que el tile no salte de alto entre estados.
    let kind_n_view: View<Msg> = match model.policy.min_attesters_of_kind {
        Some((_, n)) => slider_view(
            "min de kind",
            n as f32,
            1.0,
            5.0,
            &slider_palette,
            |phase, dv| Some(Msg::SliderMinKind(phase, dv)),
        ),
        None => label_line("(activá un kind para exigir un mínimo)", 10.0, theme.fg_muted),
    };

    let max_age_button = boton_frac(
        format!("edad máx: {}", format_max_age(model.policy.max_age_secs)),
        1.0,
        30.0,
        &button_palette_secondary(theme),
        Msg::CycleMaxAge,
    );

    let verdict_block = match model
        .selected_attestation
        .and_then(|i| model.graph.attestations().get(i).cloned())
    {
        None => column(vec![label_line(
            "seleccioná una atestación para ver el veredicto",
            12.0,
            theme.fg_muted,
        )]),
        Some(att) => {
            let cor: Corroboration =
                model
                    .graph
                    .corroboration(att.claim.subject, &att.claim.predicate, &att.claim.value);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let ok = model.graph.is_accepted_at(
                att.claim.subject,
                &att.claim.predicate,
                &att.claim.value,
                &model.policy,
                now,
            );
            let veredicto_color = if ok { theme.accent } else { theme.fg_destructive };
            let veredicto = label_line(if ok { "ACEPTA" } else { "rechaza" }, 26.0, veredicto_color);

            let eje_basico = model.policy.accepts(&cor);
            let eje_kind = match model.policy.min_attesters_of_kind {
                None => None,
                Some((kind, n)) => {
                    let count = cor
                        .attesters
                        .iter()
                        .filter(|id| {
                            model
                                .graph
                                .identity(**id)
                                .map(|i| i.kind == kind)
                                .unwrap_or(false)
                        })
                        .count();
                    Some((kind, n, count))
                }
            };
            let eje_edad: Option<(u64, u64)> = match model.policy.max_age_secs {
                None => None,
                Some(max_age) => {
                    let mas_reciente = model
                        .graph
                        .attestations()
                        .iter()
                        .filter(|a| {
                            a.claim.subject == att.claim.subject
                                && a.claim.predicate == att.claim.predicate
                                && a.claim.value == att.claim.value
                        })
                        .map(|a| a.claim.issued_at)
                        .max()
                        .unwrap_or(0);
                    Some((now.saturating_sub(mas_reciente), max_age))
                }
            };

            let mut detail: Vec<View<Msg>> = vec![
                label_line(
                    &format!("claim: {} = {}", att.claim.predicate, att.claim.value),
                    12.0,
                    theme.fg_text,
                ),
                label_line(&format!("sujeto: {}", att.claim.subject), 11.0, theme.fg_muted),
                spacer(4.0),
                label_line(
                    &format!(
                        "{}  básico: terceros {} / {} · auto {}",
                        if eje_basico { "✓" } else { "✗" },
                        cor.third_party(),
                        model.policy.min_third_party,
                        if cor.self_attested { "sí" } else { "no" }
                    ),
                    11.0,
                    if eje_basico { theme.fg_muted } else { theme.fg_destructive },
                ),
            ];
            if let Some((kind, requeridos, count)) = eje_kind {
                let pasa = count >= requeridos;
                detail.push(label_line(
                    &format!(
                        "{}  kind: {} {} / {}",
                        if pasa { "✓" } else { "✗" },
                        kind_str(kind),
                        count,
                        requeridos
                    ),
                    11.0,
                    if pasa { theme.fg_muted } else { theme.fg_destructive },
                ));
            }
            if let Some((edad, max_age)) = eje_edad {
                let pasa = edad <= max_age;
                detail.push(label_line(
                    &format!(
                        "{}  edad: {} / {} máx",
                        if pasa { "✓" } else { "✗" },
                        format_duration(edad),
                        format_duration(max_age)
                    ),
                    11.0,
                    if pasa { theme.fg_muted } else { theme.fg_destructive },
                ));
            }
            detail.push(spacer(6.0));
            detail.push(veredicto);
            column(detail)
        }
    };

    column(vec![
        spacer(8.0),
        slider,
        spacer(8.0),
        toggle,
        spacer(8.0),
        kind_button,
        kind_n_view,
        spacer(8.0),
        max_age_button,
        spacer(12.0),
        verdict_block,
        grow(empty()),
    ])
}
