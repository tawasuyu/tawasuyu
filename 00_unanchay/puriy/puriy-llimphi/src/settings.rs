//! Panel de configuración embebido de `puriy` (allichay). `Ctrl+,` lo abre como
//! modal sobre el chrome; describe las preferencias del navegador como un
//! [`Schema`] y el renderizador único las pinta con dientes y controles. Cada
//! cambio vuelve como `(FieldPath, FieldValue)` y se aplica al [`Model`] en
//! [`apply_settings_change`], que además persiste lo relevante en el Profile.
//!
//! Comparte los tipos del crate vía `use super::*` (regla #1).

use super::*;

use allichay::{EnumOption, Field, FieldPath, FieldValue, Schema, Section};
use llimphi_module_allichay::{settings_overlay, AllichayMsg};
use rimay_localize::{t, t_args};

/// Describe las preferencias actuales como un esquema editable. Se reconstruye
/// en cada frame leyendo el `Model` — los valores que muestra son los vigentes.
pub(crate) fn settings_schema(model: &Model) -> Schema {
    // Opciones de orientación de las pestañas — el corazón de esta tarea.
    let orient_opts = vec![
        EnumOption::new(TabOrientation::Horizontal.id(), t("puriy-set-orient-horizontal")),
        EnumOption::new(TabOrientation::Vertical.id(), t("puriy-set-orient-vertical")),
    ];
    // Temas: los presets de `llimphi-theme` (sus nombres son ids estables).
    let theme_opts: Vec<EnumOption> = Theme::all()
        .iter()
        .map(|th| EnumOption::new(th.name, th.name))
        .collect();

    Schema::new()
        .section(
            Section::new("pestanas", t("puriy-set-sec-tabs"))
                .icon("▦")
                .help(t("puriy-set-tabs-help"))
                .field(Field::dropdown(
                    "orientacion",
                    t("puriy-set-orientation"),
                    model.orientation.id(),
                    orient_opts,
                ))
                .field(Field::display(
                    "spaces",
                    t("puriy-set-spaces"),
                    t_args("puriy-set-spaces-count", &[("n", model.space_count().to_string().into())]),
                )),
        )
        .section(
            Section::new("apariencia", t("puriy-set-sec-appearance"))
                .icon("◐")
                .help(t("puriy-set-appearance-help"))
                .field(Field::dropdown(
                    "tema",
                    t("puriy-set-theme"),
                    model.theme.name,
                    theme_opts,
                )),
        )
}

/// Pinta el panel como modal centrado sobre la ventana. `state` es el
/// [`AllichayState`] que el `Model` guarda mientras el panel está abierto.
pub(crate) fn settings_overlay_view(model: &Model) -> View<Msg> {
    let schema = settings_schema(model);
    // Mismo criterio que los otros overlays del chrome: posicioná contra el
    // tamaño base de la ventana (`initial_size`).
    let (w, h) = Puriy::initial_size();
    settings_overlay(
        t("puriy-settings-title"),
        t("close"),
        &schema,
        &model.settings,
        &model.theme,
        (w as f32, h as f32),
        Msg::Settings,
        Msg::CloseSettings,
    )
}

/// Enruta un `AllichayMsg` del renderizador: `Change` se aplica al `Model`; el
/// resto (selección de diente, scroll, foco) muta el estado del panel.
pub(crate) fn apply_settings_msg(model: &mut Model, am: AllichayMsg) {
    match am {
        AllichayMsg::Change(path, value) => apply_settings_change(model, &path, value),
        AllichayMsg::SelectSection(i) => model.settings.select(i),
        AllichayMsg::ScrollTo(o) => model.settings.set_scroll(o),
        // Los campos son dropdowns/display: no hay foco de texto/celda/hex.
        _ => {}
    }
}

/// Aplica un cambio puntual del panel al `Model`. La ruta identifica el campo
/// (`seccion.campo`); el valor ya viene con el tipo correcto del control.
pub(crate) fn apply_settings_change(model: &mut Model, path: &FieldPath, value: FieldValue) {
    match path.to_string().as_str() {
        "pestanas.orientacion" => {
            if let Some(o) = value.as_str().and_then(TabOrientation::from_id) {
                model.orientation = o;
                persist_ui_prefs(model);
            }
        }
        "apariencia.tema" => {
            if let Some(th) = value.as_str().and_then(Theme::by_name) {
                model.theme = th;
            }
        }
        // `pestanas.spaces` es sólo lectura (display); cualquier otra ruta es
        // desconocida y se ignora.
        _ => {}
    }
}

/// Persiste orientación + spaces en el Profile (si está cableado) y graba a
/// disco. Best-effort, silencioso ante errores — igual que `persist_profile`.
pub(crate) fn persist_ui_prefs(model: &Model) {
    let Some(handle) = profile_handle() else { return };
    if let Ok(mut p) = handle.lock() {
        p.ui.orientation = model.orientation.id().to_string();
        p.ui.spaces = model
            .spaces
            .iter()
            .map(|s| puriy_core::SpacePref::new(s.name.clone(), s.icon.clone()))
            .collect();
    }
    persist_profile();
}
