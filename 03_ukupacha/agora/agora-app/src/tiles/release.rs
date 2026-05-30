//! Tile **Release** — plano de control de wawa.
//!
//! Firma un [`format::ManifiestoFirmado`]: empareja el hash de una imagen del
//! sistema con el firmante activo y su firma Ed25519. Es exactamente el sobre
//! que `apps/mudanza` empuja al kernel vía `sys_manifiesto_proponer`, y que el
//! kernel re-verifica en `wawa-kernel/src/claves.rs`. La app reusa
//! [`agora_channel::firmar_manifiesto`] / [`agora_channel::verificar_manifiesto`]
//! — el mismo código que corre el kernel, no una reimplementación.

use llimphi_theme::Theme;
use llimphi_ui::View;
use llimphi_widget_text_input::TextInputPalette;

use crate::model::{FocusedInput, Model, Msg};
use crate::ui::{
    boton_frac, bytes_to_hex, button_palette_primary, button_palette_secondary, column, empty, grow,
    input_row, label_line, row, spacer,
};

pub(crate) fn release_view(model: &Model, theme: &Theme) -> View<Msg> {
    let input_palette = TextInputPalette::from_theme(theme);

    let signer_line = format!(
        "firma: {}",
        model
            .active_signer
            .map(|id| format!("★ {id}"))
            .unwrap_or_else(|| "(ninguna — creá una identidad)".into())
    );

    let input_hash = input_row(
        &model.release_hash,
        "hash BLAKE3 del manifiesto (64 hex) …",
        model.focused_input == FocusedInput::ReleaseHash,
        &input_palette,
        Msg::Foco(FocusedInput::ReleaseHash),
    );

    let firmar = boton_frac(
        "firmar release (Enter)",
        1.0,
        32.0,
        &button_palette_primary(theme),
        Msg::FirmarRelease,
    );

    // Bloque del sobre vigente, si hay uno.
    let sobre_block: View<Msg> = match &model.release_current {
        None => label_line(
            "(sin release vigente — pegá un hash y firmá)",
            11.0,
            theme.fg_muted,
        ),
        Some(mf) => column(vec![
            label_line(
                &format!("manifiesto: {}", bytes_to_hex(&mf.manifiesto_hash)),
                10.0,
                theme.fg_text,
            ),
            label_line(&format!("autor: {}", bytes_to_hex(&mf.autor)), 10.0, theme.fg_muted),
            label_line(
                &format!("firma: {}…", &bytes_to_hex(&mf.firma)[..32]),
                10.0,
                theme.fg_muted,
            ),
        ]),
    };

    let acciones = row(
        30.0,
        vec![
            boton_frac(
                "exportar postcard →",
                0.5,
                30.0,
                &button_palette_secondary(theme),
                Msg::ExportarRelease,
            ),
            boton_frac(
                "limpiar",
                0.5,
                30.0,
                &button_palette_secondary(theme),
                Msg::LimpiarRelease,
            ),
        ],
    );

    let input_verificar = input_row(
        &model.release_paste,
        "postcard hex de un ManifiestoFirmado a verificar …",
        model.focused_input == FocusedInput::ReleasePaste,
        &input_palette,
        Msg::Foco(FocusedInput::ReleasePaste),
    );

    let verificar = boton_frac(
        "verificar pegado (Enter)",
        1.0,
        30.0,
        &button_palette_secondary(theme),
        Msg::VerificarRelease,
    );

    let status_color = if model.release_status.starts_with("✓") {
        theme.accent
    } else if model.release_status.is_empty() {
        theme.fg_muted
    } else {
        theme.fg_destructive
    };
    let status = label_line(
        if model.release_status.is_empty() {
            "el kernel honra esta firma sólo si el autor habita el AGORA_AUTH_RING"
        } else {
            &model.release_status
        },
        11.0,
        status_color,
    );

    column(vec![
        spacer(6.0),
        label_line(&signer_line, 13.0, theme.fg_text),
        spacer(6.0),
        label_line("hash del manifiesto", 10.0, theme.fg_muted),
        input_hash,
        spacer(6.0),
        firmar,
        spacer(8.0),
        sobre_block,
        spacer(6.0),
        acciones,
        spacer(10.0),
        label_line("verificar un sobre ajeno", 10.0, theme.fg_muted),
        input_verificar,
        spacer(4.0),
        verificar,
        spacer(6.0),
        status,
        grow(empty()),
    ])
}
