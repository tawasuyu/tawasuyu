//! Tile **Capacidad** — plano de control de wawa (§14.1.3).
//!
//! Concede capacidades por `(hash_bytecode, permisos)`: firma una
//! [`format::ConcesionCapacidad`] con [`agora_channel::firmar_capacidad`]. La
//! firma cubre `format::mensaje_capacidad(bytecode, permisos)` y viaja CON el
//! bytecode — ningún manifiesto puede escalar un binario más allá de lo que su
//! concesión autoriza: el kernel toma la **intersección**
//! (`format::permisos_efectivos`) contra el `AGORA_AUTH_RING`.

use format::{
    Permisos, PERMISO_ALTAVOZ, PERMISO_COMPACTAR, PERMISO_CONFIG, PERMISO_GRAFO_ESCRITURA,
    PERMISO_RAIZ, PERMISO_RED, PERMISO_TINKUY,
};
use llimphi_theme::Theme;
use llimphi_ui::View;
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_text_input::TextInputPalette;

use crate::model::{FocusedInput, Model, Msg};
use crate::ui::{
    boton_frac, bytes_to_hex, button_palette_primary, button_palette_secondary, column, grow,
    input_row, label_line, row, spacer,
};

/// Catálogo de bits concedibles: cada uno es una capacidad física del kernel.
/// El orden y las etiquetas espejan las constantes `PERMISO_*` de `format`.
pub(crate) const CATALOGO: [(Permisos, &str); 7] = [
    (PERMISO_RED, "red (frames + objetos por hash)"),
    (PERMISO_GRAFO_ESCRITURA, "grafo: escritura"),
    (PERMISO_RAIZ, "raíz (reanclar el grafo)"),
    (PERMISO_ALTAVOZ, "altavoz (bocina)"),
    (PERMISO_CONFIG, "config (proponer idioma/tema)"),
    (PERMISO_COMPACTAR, "compactar (GC explícito)"),
    (PERMISO_TINKUY, "tinkuy (motor embebido)"),
];

/// Resumen textual de un bitfield de permisos (`red·raíz` o `—`).
pub(crate) fn resumen_permisos(p: Permisos) -> String {
    let activos: Vec<&str> = CATALOGO
        .iter()
        .filter(|(bit, _)| p & bit != 0)
        .map(|(_, label)| label.split(' ').next().unwrap_or(""))
        .collect();
    if activos.is_empty() {
        "— (sin permisos)".into()
    } else {
        activos.join(" · ")
    }
}

pub(crate) fn capacidad_view(model: &Model, theme: &Theme) -> View<Msg> {
    let input_palette = TextInputPalette::from_theme(theme);
    let list_palette = ListPalette::from_theme(theme);

    let signer_line = format!(
        "concede: {}",
        model
            .active_signer
            .map(|id| format!("★ {id}"))
            .unwrap_or_else(|| "(ninguna — creá una identidad)".into())
    );

    let input_bytecode = input_row(
        &model.cap_bytecode,
        "hash BLAKE3 del bytecode WASM (64 hex) …",
        model.focused_input == FocusedInput::CapBytecode,
        &input_palette,
        Msg::Foco(FocusedInput::CapBytecode),
    );

    // Toggles de permisos como lista con check ☑/☐.
    let rows: Vec<ListRow<Msg>> = CATALOGO
        .iter()
        .map(|(bit, label)| {
            let on = model.cap_permisos & bit != 0;
            ListRow {
                label: format!("{}  {label}", if on { "☑" } else { "☐" }),
                selected: false,
                on_click: Msg::ToggleCapPermiso(*bit),
            }
        })
        .collect();
    let permisos_list = list_view(ListSpec {
        rows,
        total: CATALOGO.len(),
        caption: Some(format!("permisos: {}", resumen_permisos(model.cap_permisos))),
        truncated_hint: None,
        row_height: 22.0,
        palette: list_palette,
    });

    let conceder = boton_frac(
        "conceder capacidad (Enter)",
        1.0,
        32.0,
        &button_palette_primary(theme),
        Msg::FirmarCapacidad,
    );

    let sobre_block: View<Msg> = match &model.cap_current {
        None => label_line(
            "(sin concesión vigente — pegá un hash, elegí permisos y concedé)",
            11.0,
            theme.fg_muted,
        ),
        Some(c) => column(vec![
            label_line(&format!("bytecode: {}", bytes_to_hex(&c.bytecode)), 10.0, theme.fg_text),
            label_line(&format!("permisos: 0b{:07b} · {}", c.permisos, resumen_permisos(c.permisos)), 10.0, theme.fg_muted),
            label_line(&format!("autor: {}", bytes_to_hex(&c.autor)), 10.0, theme.fg_muted),
            label_line(&format!("firma: {}…", &bytes_to_hex(&c.firma)[..32]), 10.0, theme.fg_muted),
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
                Msg::ExportarCapacidad,
            ),
            boton_frac(
                "limpiar",
                0.5,
                30.0,
                &button_palette_secondary(theme),
                Msg::LimpiarCapacidad,
            ),
        ],
    );

    let input_verificar = input_row(
        &model.cap_paste,
        "postcard hex de una ConcesionCapacidad a verificar …",
        model.focused_input == FocusedInput::CapPaste,
        &input_palette,
        Msg::Foco(FocusedInput::CapPaste),
    );

    let verificar = boton_frac(
        "verificar pegado (Enter)",
        1.0,
        30.0,
        &button_palette_secondary(theme),
        Msg::VerificarCapacidad,
    );

    let status_color = if model.cap_status.starts_with("✓") {
        theme.accent
    } else if model.cap_status.is_empty() {
        theme.fg_muted
    } else {
        theme.fg_destructive
    };
    let status = label_line(
        if model.cap_status.is_empty() {
            "la firma cierra matemáticamente; el anillo lo decide el kernel"
        } else {
            &model.cap_status
        },
        11.0,
        status_color,
    );

    column(vec![
        spacer(6.0),
        label_line(&signer_line, 13.0, theme.fg_text),
        spacer(6.0),
        label_line("hash del bytecode", 10.0, theme.fg_muted),
        input_bytecode,
        spacer(6.0),
        grow(permisos_list),
        spacer(6.0),
        conceder,
        spacer(8.0),
        sobre_block,
        spacer(6.0),
        acciones,
        spacer(10.0),
        label_line("verificar una concesión ajena", 10.0, theme.fg_muted),
        input_verificar,
        spacer(4.0),
        verificar,
        spacer(6.0),
        status,
    ])
}
