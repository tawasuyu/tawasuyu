//! Tile **Compositor**: con sujeto y firmante elegidos, edita predicado y
//! valor y firma una atestación (Enter).

use llimphi_theme::Theme;
use llimphi_ui::View;
use llimphi_widget_text_input::TextInputPalette;

use crate::model::{ComposeField, FocusedInput, Model, Msg};
use crate::ui::{boton_frac, button_palette_primary, column, empty, grow, input_row, label_line, spacer};

pub(crate) fn compositor_view(model: &Model, theme: &Theme) -> View<Msg> {
    let input_palette = TextInputPalette::from_theme(theme);

    let signer_line = format!(
        "yo: {}",
        model
            .active_signer
            .map(|id| format!("★ {id}"))
            .unwrap_or_else(|| "(ninguna — creá una identidad)".into())
    );
    let subject_line = format!(
        "sobre: {}",
        model
            .focused_subject
            .map(|id| format!("{id}"))
            .unwrap_or_else(|| "(elegí una identidad en el tile de la izquierda)".into())
    );

    let input_predicate = input_row(
        &model.compose_predicate,
        "nacionalidad / miembro-de / habilidad …",
        model.focused_input == FocusedInput::Compose(ComposeField::Predicate),
        &input_palette,
        Msg::Foco(FocusedInput::Compose(ComposeField::Predicate)),
    );
    let input_value = input_row(
        &model.compose_value,
        "venezolana / El Valle / soldadura …",
        model.focused_input == FocusedInput::Compose(ComposeField::Value),
        &input_palette,
        Msg::Foco(FocusedInput::Compose(ComposeField::Value)),
    );

    let firmar = boton_frac(
        "atestar (Enter)",
        1.0,
        34.0,
        &button_palette_primary(theme),
        Msg::Atestar,
    );

    let status_color = if model.compose_status.starts_with("atestación") {
        theme.accent
    } else if model.compose_status.is_empty() {
        theme.fg_muted
    } else {
        theme.fg_destructive
    };
    let status = label_line(
        if model.compose_status.is_empty() {
            "Tab cicla campos · Enter firma"
        } else {
            &model.compose_status
        },
        11.0,
        status_color,
    );

    column(vec![
        spacer(6.0),
        label_line(&signer_line, 13.0, theme.fg_text),
        label_line(&subject_line, 13.0, theme.fg_text),
        spacer(8.0),
        label_line("predicado", 10.0, theme.fg_muted),
        input_predicate,
        spacer(4.0),
        label_line("valor", 10.0, theme.fg_muted),
        input_value,
        spacer(8.0),
        firmar,
        spacer(6.0),
        status,
        grow(empty()),
    ])
}
