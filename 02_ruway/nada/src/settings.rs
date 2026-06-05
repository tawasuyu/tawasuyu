//! Panel de configuración de `nada`, embebido como modal sobre la ventana
//! del editor (la tecla `Ctrl+,` lo abre; Esc / clic en el scrim lo cierra).
//!
//! Es el primer consumidor de `llimphi_module_allichay::settings_overlay`: en
//! vez de un panel a mano, `nada` describe sus preferencias como un
//! [`allichay::Schema`] y el renderizador único las pinta con dientes y
//! controles. Cada cambio vuelve como `(FieldPath, FieldValue)` y se aplica al
//! `Model` en [`apply_settings_change`]. Las preferencias de `nada` viven en su
//! propio `Model` (tema, formateo al guardar, diagnósticos demo), así que el
//! schema se reconstruye en cada `view` a partir del estado actual.

use super::*;

use allichay::{EnumOption, Field, FieldValue, Schema, Section};
use llimphi_module_allichay::settings_overlay;

/// Describe las preferencias actuales de `nada` como un esquema editable. Se
/// reconstruye en cada frame leyendo el `Model` — los valores que muestra son
/// los vigentes.
pub(crate) fn settings_schema(model: &Model) -> Schema {
    let t = rimay_localize::t;
    // Las opciones de tema son los presets de `llimphi-theme` (sus nombres son
    // ids estables — los mismos que usa el theme-switcher). Son ≤ 4, así que el
    // renderer los pinta como botones segmentados.
    let theme_opts: Vec<EnumOption> = Theme::all()
        .iter()
        .map(|th| EnumOption::new(th.name, th.name))
        .collect();

    Schema::new()
        .section(
            Section::new("apariencia", t("nada-settings-appearance"))
                .icon("◐")
                .field(Field::dropdown(
                    "tema",
                    t("nada-settings-theme"),
                    model.theme.name,
                    theme_opts,
                )),
        )
        .section(
            Section::new("editor", t("nada-settings-editor"))
                .icon("✎")
                .field(Field::toggle(
                    "fmt_on_save",
                    t("nada-settings-fmt-on-save"),
                    model.format_on_save,
                ))
                .field(Field::toggle(
                    "demo_diag",
                    t("nada-settings-demo-diag"),
                    model.demo_lsp,
                ))
                .field(Field::display(
                    "lsp",
                    t("nada-settings-lsp"),
                    model.lsp_label.clone(),
                )),
        )
}

/// Pinta el panel como modal centrado sobre la ventana. `state` es el
/// [`AllichayState`] que el `Model` guarda mientras el panel está abierto.
pub(crate) fn settings_overlay_view(model: &Model, state: &AllichayState) -> View<Msg> {
    let schema = settings_schema(model);
    settings_overlay(
        rimay_localize::t("settings"),
        rimay_localize::t("close"),
        &schema,
        state,
        &model.theme,
        (model.win_w, model.win_h),
        Msg::Settings,
        Msg::SettingsClose,
    )
}

/// Aplica un cambio puntual del panel al `Model`. La ruta identifica el campo
/// (`seccion.campo`); el valor ya viene con el tipo correcto del control.
pub(crate) fn apply_settings_change(model: &mut Model, path: &FieldPath, value: FieldValue) {
    match path.to_string().as_str() {
        "apariencia.tema" => {
            if let Some(th) = value.as_str().and_then(Theme::by_name) {
                model.theme = th;
                model.status = format!("✓ tema: {}", model.theme.name);
            }
        }
        "editor.fmt_on_save" => {
            if let Some(b) = value.as_bool() {
                model.format_on_save = b;
            }
        }
        "editor.demo_diag" => {
            if let Some(b) = value.as_bool() {
                model.demo_lsp = b;
            }
        }
        // El servidor de lenguaje es sólo lectura (se elige por CLI): no emite
        // Change. Cualquier otra ruta es desconocida y se ignora.
        _ => {}
    }
}
