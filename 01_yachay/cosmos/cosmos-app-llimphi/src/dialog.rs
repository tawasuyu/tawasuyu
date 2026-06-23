//! Diálogos modales de creación: **contacto** y **carta**.
//!
//! Rescatado del cosmos GPUI (cosmos-tree, "Fase 2 — CRUD UX", borrado en
//! la migración a Llimphi 2026-05-26): el form de carta con los campos
//! mínimos de `StoredBirthData` y el **atlas de ciudades** que autocompleta
//! lat/lon/tz al elegir un lugar de nacimiento.
//!
//! Se renderea como overlay (`App::view_overlay`): un scrim a pantalla
//! completa + una card centrada. Un solo `TextInputState` en el `Model`
//! edita el campo enfocado; el valor vive en el form y se escribe en cada
//! tecla. La confirmación valida/parsea y crea en el store.

use cosmos_model::{ChartKind, ContactId, GroupId};

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_calendar::{calendar_view, CalendarPalette, CalendarSpec, WeekStart};
use llimphi_widget_panel::{panel_signature_painter, PanelStyle};
use llimphi_widget_text_input::{text_input_view, TextInputPalette};

use crate::chrome::kind_label_es;
use crate::glyphs::{self, Icon};
use crate::model::{Model, Msg};

/// Tipos de carta ofrecidos en el select, en orden de uso. `Natal`
/// («radix») es el default. `Mundane` queda fuera — es la rama «Hoy».
pub(crate) const CHART_KINDS: &[ChartKind] = &[
    ChartKind::Natal,
    ChartKind::Transit,
    ChartKind::SolarReturn,
    ChartKind::LunarReturn,
    ChartKind::SecondaryProgression,
    ChartKind::TertiaryProgression,
    ChartKind::MinorProgression,
    ChartKind::SolarArc,
    ChartKind::PrimaryDirection,
    ChartKind::Profection,
    ChartKind::Synastry,
    ChartKind::Composite,
    ChartKind::Davison,
];

/// Preset de ciudad: autocompleta lat/lon/tz al elegirlo. TZ es la zona
/// estándar (sin DST). Rescatado del cosmos GPUI.
#[derive(Clone, Debug)]
pub(crate) struct CityPreset {
    pub name: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub tz: i32,
}

/// Campo del diálogo con foco de teclado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DialogField {
    #[default]
    Name,
    /// Combobox editable de contacto (texto = nombre tecleado o elegido).
    Contact,
    Label,
    Date,
    Time,
    City,
    Lat,
    Lon,
}

/// A quién aplica un formulario de ubicación «Hoy»: la ubicación fija del
/// usuario, o una carta extra del día por coordenadas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HoyTarget {
    User,
    Extra,
}

/// Estado de un diálogo abierto.
pub(crate) enum Dialog {
    NewContact(NewContactForm),
    NewChart(NewChartForm),
    HoyLoc(HoyLocForm),
}

/// Formulario de ubicación de la rama «Hoy» (sin fecha — la carta es del
/// instante actual). El usuario elige una ciudad del atlas (autocompleta
/// lat/lon) o teclea las coordenadas a mano.
pub(crate) struct HoyLocForm {
    pub target: HoyTarget,
    pub label: String,
    pub city_query: String,
    pub place: String,
    /// Coordenadas como texto editable (`""` hasta elegir ciudad o teclear).
    pub lat: String,
    pub lon: String,
}

pub(crate) struct NewContactForm {
    pub group: Option<GroupId>,
    pub name: String,
}

pub(crate) struct NewChartForm {
    /// Contacto destino, si ya existe. `None` = se creará uno nuevo con el
    /// nombre tecleado en `contact_query` al confirmar.
    pub contact: Option<ContactId>,
    /// Grupo donde aterriza un contacto nuevo (el del nodo de origen).
    pub group: Option<GroupId>,
    /// Texto del combobox de contacto: nombre tecleado o el del elegido.
    pub contact_query: String,
    /// Tipo de carta. Default `Natal` (radix).
    pub kind: ChartKind,
    pub label: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    /// `HH:MM`.
    pub time: String,
    pub city_query: String,
    pub place: String,
    pub lat: f64,
    pub lon: f64,
    pub tz: i32,
    /// Lista inline de tipos de carta desplegada.
    pub kind_open: bool,
    /// Calendario inline desplegado.
    pub cal_open: bool,
    /// Mes/año en foco del calendario inline.
    pub cal_year: i32,
    pub cal_month: u32,
}

impl Dialog {
    /// Lee el valor textual del campo `f`.
    pub(crate) fn field(&self, f: DialogField) -> String {
        match (self, f) {
            (Dialog::NewContact(c), DialogField::Name) => c.name.clone(),
            (Dialog::NewChart(c), DialogField::Contact) => c.contact_query.clone(),
            (Dialog::NewChart(c), DialogField::Label) => c.label.clone(),
            (Dialog::NewChart(c), DialogField::Date) => c.date.clone(),
            (Dialog::NewChart(c), DialogField::Time) => c.time.clone(),
            (Dialog::NewChart(c), DialogField::City) => c.city_query.clone(),
            (Dialog::HoyLoc(c), DialogField::Label) => c.label.clone(),
            (Dialog::HoyLoc(c), DialogField::City) => c.city_query.clone(),
            (Dialog::HoyLoc(c), DialogField::Lat) => c.lat.clone(),
            (Dialog::HoyLoc(c), DialogField::Lon) => c.lon.clone(),
            _ => String::new(),
        }
    }
    /// Escribe `v` en el campo `f`.
    pub(crate) fn set_field(&mut self, f: DialogField, v: String) {
        match (self, f) {
            (Dialog::NewContact(c), DialogField::Name) => c.name = v,
            (Dialog::NewChart(c), DialogField::Contact) => {
                // Teclear en el combobox de contacto invalida la elección
                // previa: vuelve a modo «crear nuevo» hasta que se elija.
                c.contact = None;
                c.contact_query = v;
            }
            (Dialog::NewChart(c), DialogField::Label) => c.label = v,
            (Dialog::NewChart(c), DialogField::Date) => c.date = v,
            (Dialog::NewChart(c), DialogField::Time) => c.time = v,
            (Dialog::NewChart(c), DialogField::City) => c.city_query = v,
            (Dialog::HoyLoc(c), DialogField::Label) => c.label = v,
            (Dialog::HoyLoc(c), DialogField::City) => c.city_query = v,
            (Dialog::HoyLoc(c), DialogField::Lat) => c.lat = v,
            (Dialog::HoyLoc(c), DialogField::Lon) => c.lon = v,
            _ => {}
        }
    }
}

/// Ciudades que matchean la query (case-insensitive, substring).
pub(crate) fn city_matches(query: &str) -> Vec<(usize, &'static CityPreset)> {
    let q = query.trim().to_lowercase();
    CITY_PRESETS
        .iter()
        .enumerate()
        .filter(|(_, c)| q.is_empty() || c.name.to_lowercase().contains(&q))
        .take(8)
        .collect()
}

// =====================================================================
// Render
// =====================================================================

pub(crate) fn dialog_overlay(model: &Model, theme: &Theme) -> Option<View<Msg>> {
    let dialog = model.dialog.as_ref()?;
    let (title, body): (&str, Vec<View<Msg>>) = match dialog {
        Dialog::NewContact(_) => ("Nuevo contacto", contact_body(model, theme)),
        Dialog::NewChart(_) => ("Nueva carta", chart_body(model, theme)),
        Dialog::HoyLoc(f) => (
            match f.target {
                HoyTarget::User => "¿Dónde estoy?",
                HoyTarget::Extra => "Carta de hoy",
            },
            hoy_body(model, theme),
        ),
    };

    // Card centrada.
    let mut kids: Vec<View<Msg>> = Vec::new();
    kids.push(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(26.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            ..Default::default()
        })
        .text_aligned(title.to_string(), 14.0, theme.fg_text, Alignment::Start)]),
    );
    kids.extend(body);
    kids.push(dialog_buttons(theme));

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(420.0_f32),
            height: Dimension::auto(),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(14.0_f32),
            bottom: length(14.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .paint_with(panel_signature_painter(PanelStyle::from_theme_large(theme)))
    .radius(PanelStyle::from_theme_large(theme).radius)
    .clip(true)
    .children(kids);

    // Scrim a pantalla completa. El diálogo es **bloqueante**: el click
    // afuera NO cierra (sólo los botones Cancelar/Crear o Esc). El
    // `on_click` neutro absorbe el clic para que no caiga al lienzo detrás.
    Some(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(scrim(theme))
        .on_click(Msg::DialogNop)
        .children(vec![card]),
    )
}

fn scrim(theme: &Theme) -> llimphi_ui::llimphi_raster::peniko::Color {
    let [r, g, b, _] = theme.bg_app.components;
    llimphi_ui::llimphi_raster::peniko::Color::new([r, g, b, 0.6])
}

fn contact_body(model: &Model, theme: &Theme) -> Vec<View<Msg>> {
    vec![field_row(model, theme, "Nombre", DialogField::Name)]
}

/// Contactos existentes que matchean la query (case-insensitive,
/// substring), derivados del snapshot del árbol. Hasta 6.
pub(crate) fn contact_matches(model: &Model, query: &str) -> Vec<(ContactId, String)> {
    let q = query.trim().to_lowercase();
    model
        .nav_nodes
        .iter()
        .filter(|n| n.kind == crate::library::NavKind::Contact)
        .filter_map(|n| crate::library::parse_contact_key(&n.key).map(|id| (id, n.label.clone())))
        .filter(|(_, label)| q.is_empty() || label.to_lowercase().contains(&q))
        .take(6)
        .collect()
}

fn chart_body(model: &Model, theme: &Theme) -> Vec<View<Msg>> {
    let form = match &model.dialog {
        Some(Dialog::NewChart(c)) => c,
        _ => return Vec::new(),
    };
    let mut rows: Vec<View<Msg>> = Vec::new();

    // --- Contacto: combobox editable (teclear nombre / elegir existente) ---
    rows.push(field_row(model, theme, "Contacto", DialogField::Contact));
    if model.dialog_field == DialogField::Contact {
        let matches = contact_matches(model, &form.contact_query);
        for (id, name) in &matches {
            let chosen = form.contact == Some(*id);
            rows.push(picker_row(
                theme,
                name,
                chosen,
                glyphs::contact_icon_view(13.0),
                Msg::DialogPickContact(*id),
            ));
        }
        // Opción de crear uno nuevo con el texto tecleado, si no coincide
        // exactamente con un contacto existente.
        let q = form.contact_query.trim();
        if !q.is_empty() && !matches.iter().any(|(_, n)| n.eq_ignore_ascii_case(q)) {
            rows.push(picker_row(
                theme,
                &format!("Crear contacto «{q}»"),
                false,
                glyphs::icon_view(Icon::Plus, 13.0, theme.accent),
                Msg::DialogFocus(DialogField::Contact),
            ));
        }
    }

    // --- Tipo de carta: select (default radix) ---
    rows.push(kind_select_row(form, theme));
    if form.kind_open {
        for k in CHART_KINDS {
            let chosen = *k == form.kind;
            rows.push(picker_row(
                theme,
                kind_label_es(*k),
                chosen,
                glyphs::chart_kind_colored(*k, 13.0),
                Msg::DialogSetKind(*k),
            ));
        }
    }

    // --- Etiqueta ---
    rows.push(field_row(model, theme, "Etiqueta", DialogField::Label));

    // --- Fecha: texto editable + calendario amigable ---
    rows.push(date_row(model, theme));
    if form.cal_open {
        rows.push(calendar_block(form, theme));
    }

    // --- Hora: texto editable + steppers ---
    rows.push(time_row(model, theme));

    // --- Ciudad ---
    rows.push(field_row(model, theme, "Ciudad", DialogField::City));
    // Lista de ciudades que matchean (al editar el campo Ciudad).
    if model.dialog_field == DialogField::City {
        if let Some(Dialog::NewChart(c)) = &model.dialog {
            for (idx, city) in city_matches(&c.city_query) {
                rows.push(
                    View::new(Style {
                        size: Size {
                            width: percent(1.0_f32),
                            height: length(22.0_f32),
                        },
                        flex_shrink: 0.0,
                        padding: Rect {
                            left: length(10.0_f32),
                            right: length(8.0_f32),
                            top: length(0.0_f32),
                            bottom: length(0.0_f32),
                        },
                        align_items: Some(AlignItems::Center),
                        ..Default::default()
                    })
                    .hover_fill(theme.bg_row_hover)
                    .radius(3.0)
                    .on_click(Msg::DialogPickCity(idx))
                    .children(vec![View::new(Style {
                        size: Size {
                            width: percent(1.0_f32),
                            height: Dimension::auto(),
                        },
                        ..Default::default()
                    })
                    .text_aligned(city.name.to_string(), 11.0, theme.fg_muted, Alignment::Start)]),
                );
            }
        }
    }
    // Resumen del lugar elegido.
    if let Some(Dialog::NewChart(c)) = &model.dialog {
        if !c.place.is_empty() {
            rows.push(
                View::new(Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(18.0_f32),
                    },
                    flex_shrink: 0.0,
                    align_items: Some(AlignItems::Center),
                    ..Default::default()
                })
                .children(vec![View::new(Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: Dimension::auto(),
                    },
                    ..Default::default()
                })
                .text_aligned(
                    format!(
                        "{}  ·  {:.2}°, {:.2}°  ·  UTC{:+}",
                        c.place,
                        c.lat,
                        c.lon,
                        c.tz as f32 / 60.0
                    ),
                    10.0,
                    theme.accent,
                    Alignment::Start,
                )]),
            );
        }
    }
    rows
}

fn hoy_body(model: &Model, theme: &Theme) -> Vec<View<Msg>> {
    let mut rows = vec![
        field_row(model, theme, "Etiqueta", DialogField::Label),
        field_row(model, theme, "Ciudad", DialogField::City),
    ];
    // Lista de ciudades que matchean (al editar el campo Ciudad).
    if model.dialog_field == DialogField::City {
        if let Some(Dialog::HoyLoc(c)) = &model.dialog {
            for (idx, city) in city_matches(&c.city_query) {
                rows.push(
                    View::new(Style {
                        size: Size {
                            width: percent(1.0_f32),
                            height: length(22.0_f32),
                        },
                        flex_shrink: 0.0,
                        padding: Rect {
                            left: length(10.0_f32),
                            right: length(8.0_f32),
                            top: length(0.0_f32),
                            bottom: length(0.0_f32),
                        },
                        align_items: Some(AlignItems::Center),
                        ..Default::default()
                    })
                    .hover_fill(theme.bg_row_hover)
                    .radius(3.0)
                    .on_click(Msg::DialogPickCity(idx))
                    .children(vec![View::new(Style {
                        size: Size {
                            width: percent(1.0_f32),
                            height: Dimension::auto(),
                        },
                        ..Default::default()
                    })
                    .text_aligned(city.name.to_string(), 11.0, theme.fg_muted, Alignment::Start)]),
                );
            }
        }
    }
    // Coordenadas (editables a mano o autocompletadas por la ciudad).
    rows.push(field_row(model, theme, "Latitud", DialogField::Lat));
    rows.push(field_row(model, theme, "Longitud", DialogField::Lon));
    rows
}

/// Celda de etiqueta de ancho fijo a la izquierda de una fila.
fn label_cell(theme: &Theme, label: &str) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(112.0_f32),
            height: Dimension::auto(),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(label.to_string(), 12.0, theme.fg_muted, Alignment::Start)
}

/// Slot elástico de input: si el `field` tiene el foco usa el `dialog_input`
/// vivo; si no, muestra su valor (clickeable para enfocar).
fn input_slot(model: &Model, theme: &Theme, field: DialogField) -> View<Msg> {
    let focused = model.dialog_field == field;
    let input: View<Msg> = if focused {
        text_input_view(
            &model.dialog_input,
            "",
            true,
            &TextInputPalette::from_theme(theme),
            Msg::DialogFocus(field),
        )
    } else {
        let val = model
            .dialog
            .as_ref()
            .map(|d| d.field(field))
            .unwrap_or_default();
        View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(0.0_f32),
                height: length(26.0_f32),
            },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .radius(4.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::DialogFocus(field))
        .children(vec![View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            ..Default::default()
        })
        .text_aligned(val, 12.0, theme.fg_text, Alignment::Start)])
    };
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: length(28.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![input])
}

/// Fila horizontal de la card: etiqueta + hijos (input + adornos).
fn dialog_row(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(30.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

/// Una fila etiqueta + campo de texto.
fn field_row(model: &Model, theme: &Theme, label: &str, field: DialogField) -> View<Msg> {
    dialog_row(vec![label_cell(theme, label), input_slot(model, theme, field)])
}

/// Botoncito cuadrado con un icono (toggle de calendario, stepper, etc.).
fn icon_button(theme: &Theme, icon: Icon, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(28.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(msg)
    .children(vec![glyphs::icon_view(icon, 13.0, theme.fg_text)])
}

/// Fila de elección dentro de un desplegable inline (contacto / tipo de
/// carta): icono + label + check si está elegida.
fn picker_row(theme: &Theme, label: &str, chosen: bool, icon: View<Msg>, msg: Msg) -> View<Msg> {
    let check = View::new(Style {
        size: Size {
            width: length(16.0_f32),
            height: Dimension::auto(),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(
        if chosen { "✓".to_string() } else { String::new() },
        11.0,
        theme.accent,
        Alignment::Center,
    );
    let icon_cell = View::new(Style {
        size: Size {
            width: length(18.0_f32),
            height: length(18.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![icon]);
    let txt = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: Dimension::auto(),
        },
        ..Default::default()
    })
    .text_aligned(label.to_string(), 11.5, theme.fg_text, Alignment::Start);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(5.0_f32),
            height: length(0.0_f32),
        },
        padding: Rect {
            left: length(118.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .radius(4.0)
    .on_click(msg)
    .children(vec![check, icon_cell, txt])
}

/// Fila select del tipo de carta: etiqueta + disparador con el tipo actual
/// (icono + nombre) y chevron. Click despliega/cierra la lista inline.
fn kind_select_row(form: &NewChartForm, theme: &Theme) -> View<Msg> {
    let chevron = if form.kind_open { "\u{25B4}" } else { "\u{25BE}" };
    let trigger = View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: length(28.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::DialogToggleKind)
    .children(vec![
        View::new(Style {
            size: Size {
                width: length(16.0_f32),
                height: length(16.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .children(vec![glyphs::chart_kind_colored(form.kind, 13.0)]),
        View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(0.0_f32),
                height: Dimension::auto(),
            },
            ..Default::default()
        })
        .text_aligned(kind_label_es(form.kind).to_string(), 12.0, theme.fg_text, Alignment::Start),
        View::new(Style {
            size: Size {
                width: length(14.0_f32),
                height: Dimension::auto(),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .text_aligned(chevron.to_string(), 10.0, theme.fg_muted, Alignment::Center),
    ]);
    dialog_row(vec![label_cell(theme, "Tipo"), trigger])
}

/// Fila de fecha: texto editable `AAAA-MM-DD` + botón que abre el
/// calendario inline.
fn date_row(model: &Model, theme: &Theme) -> View<Msg> {
    dialog_row(vec![
        label_cell(theme, "Fecha"),
        input_slot(model, theme, DialogField::Date),
        icon_button(theme, Icon::Grid, Msg::DialogToggleCalendar),
    ])
}

/// Fila de hora: texto editable `HH:MM` + steppers hora/minuto.
fn time_row(model: &Model, theme: &Theme) -> View<Msg> {
    let stepper = |up: Icon, down: Icon, up_msg: Msg, down_msg: Msg| -> View<Msg> {
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: length(22.0_f32),
                height: length(28.0_f32),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .children(vec![
            mini_step(theme, up, up_msg),
            mini_step(theme, down, down_msg),
        ])
    };
    dialog_row(vec![
        label_cell(theme, "Hora"),
        input_slot(model, theme, DialogField::Time),
        // Hora ±1
        stepper(
            Icon::ArrowUp,
            Icon::ArrowDown,
            Msg::DialogTimeStep(true, 1),
            Msg::DialogTimeStep(true, -1),
        ),
        // Minuto ±1
        stepper(
            Icon::ArrowUp,
            Icon::ArrowDown,
            Msg::DialogTimeStep(false, 1),
            Msg::DialogTimeStep(false, -1),
        ),
    ])
}

/// Medio-botón de stepper (mitad superior o inferior).
fn mini_step(theme: &Theme, icon: Icon, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(13.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(3.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(msg)
    .children(vec![glyphs::icon_view(icon, 9.0, theme.fg_muted)])
}

/// Calendario inline (centrado) para elegir el día del mes en foco.
fn calendar_block(form: &NewChartForm, theme: &Theme) -> View<Msg> {
    let selected = parse_naive_date(&form.date);
    let today = chrono::Local::now().date_naive();
    let cal = calendar_view(CalendarSpec {
        view_year: form.cal_year,
        view_month: form.cal_month,
        selected,
        today: Some(today),
        week_start: WeekStart::Monday,
        palette: CalendarPalette::from_theme(theme),
        on_select: std::sync::Arc::new(|d: chrono::NaiveDate| {
            use chrono::Datelike;
            Msg::DialogCalPick(d.year(), d.month(), d.day())
        }),
        on_view_change: std::sync::Arc::new(|y, m| Msg::DialogCalView(y, m)),
    });
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(248.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![cal])
}

/// Parsea `AAAA-MM-DD` a `NaiveDate` (None si inválida).
fn parse_naive_date(s: &str) -> Option<chrono::NaiveDate> {
    let p: Vec<&str> = s.trim().split('-').collect();
    if p.len() != 3 {
        return None;
    }
    chrono::NaiveDate::from_ymd_opt(p[0].trim().parse().ok()?, p[1].trim().parse().ok()?, p[2].trim().parse().ok()?)
}

fn dialog_buttons(theme: &Theme) -> View<Msg> {
    let btn = |label: &str, icon: Icon, msg: Msg, accent: bool| -> View<Msg> {
        let fg = if accent { theme.bg_app } else { theme.fg_text };
        let bg = if accent { theme.accent } else { theme.bg_panel };
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: Dimension::auto(),
                height: length(28.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            gap: Size {
                width: length(5.0_f32),
                height: length(0.0_f32),
            },
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(bg)
        .radius(5.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(msg)
        .children(vec![
            glyphs::icon_view(icon, 13.0, fg),
            View::new(Style {
                size: Size {
                    width: Dimension::auto(),
                    height: Dimension::auto(),
                },
                ..Default::default()
            })
            .text_aligned(label.to_string(), 12.0, fg, Alignment::Center),
        ])
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(30.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::End),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        btn("Cancelar", Icon::Close, Msg::DialogCancel, false),
        btn("Crear", Icon::Plus, Msg::DialogConfirm, true),
    ])
}

/// Atlas hardcoded — ciudades canónicas que cubren la mayoría de casos.
/// (Rescatado de `cosmos-tree::default_city_presets`.)
pub(crate) const CITY_PRESETS: &[CityPreset] = &[
    CityPreset { name: "Buenos Aires, AR", lat: -34.6037, lon: -58.3816, tz: -180 },
    CityPreset { name: "Córdoba, AR", lat: -31.4201, lon: -64.1888, tz: -180 },
    CityPreset { name: "Rosario, AR", lat: -32.9587, lon: -60.6930, tz: -180 },
    CityPreset { name: "Mendoza, AR", lat: -32.8908, lon: -68.8272, tz: -180 },
    CityPreset { name: "Caracas, VE", lat: 10.4806, lon: -66.9036, tz: -240 },
    CityPreset { name: "Maracaibo, VE", lat: 10.6427, lon: -71.6125, tz: -240 },
    CityPreset { name: "Valencia, VE", lat: 10.1620, lon: -68.0078, tz: -240 },
    CityPreset { name: "Bogotá, CO", lat: 4.7110, lon: -74.0721, tz: -300 },
    CityPreset { name: "Medellín, CO", lat: 6.2442, lon: -75.5812, tz: -300 },
    CityPreset { name: "Cali, CO", lat: 3.4516, lon: -76.5320, tz: -300 },
    CityPreset { name: "Lima, PE", lat: -12.0464, lon: -77.0428, tz: -300 },
    CityPreset { name: "Cusco, PE", lat: -13.5319, lon: -71.9675, tz: -300 },
    CityPreset { name: "Santiago, CL", lat: -33.4489, lon: -70.6693, tz: -240 },
    CityPreset { name: "Valparaíso, CL", lat: -33.0472, lon: -71.6127, tz: -240 },
    CityPreset { name: "Quito, EC", lat: -0.1807, lon: -78.4678, tz: -300 },
    CityPreset { name: "Guayaquil, EC", lat: -2.1709, lon: -79.9224, tz: -300 },
    CityPreset { name: "Montevideo, UY", lat: -34.9011, lon: -56.1645, tz: -180 },
    CityPreset { name: "Asunción, PY", lat: -25.2637, lon: -57.5759, tz: -240 },
    CityPreset { name: "La Paz, BO", lat: -16.4897, lon: -68.1193, tz: -240 },
    CityPreset { name: "Ciudad de México", lat: 19.4326, lon: -99.1332, tz: -360 },
    CityPreset { name: "Guadalajara, MX", lat: 20.6597, lon: -103.3496, tz: -360 },
    CityPreset { name: "Monterrey, MX", lat: 25.6866, lon: -100.3161, tz: -360 },
    CityPreset { name: "Habana, CU", lat: 23.1136, lon: -82.3666, tz: -300 },
    CityPreset { name: "San Juan, PR", lat: 18.4655, lon: -66.1057, tz: -240 },
    CityPreset { name: "San José, CR", lat: 9.9281, lon: -84.0907, tz: -360 },
    CityPreset { name: "Panamá, PA", lat: 8.9824, lon: -79.5199, tz: -300 },
    CityPreset { name: "San Salvador, SV", lat: 13.6929, lon: -89.2182, tz: -360 },
    CityPreset { name: "Guatemala, GT", lat: 14.6349, lon: -90.5069, tz: -360 },
    CityPreset { name: "Tegucigalpa, HN", lat: 14.0723, lon: -87.1921, tz: -360 },
    CityPreset { name: "Managua, NI", lat: 12.1149, lon: -86.2362, tz: -360 },
    CityPreset { name: "Santo Domingo, DO", lat: 18.4861, lon: -69.9312, tz: -240 },
    CityPreset { name: "São Paulo, BR", lat: -23.5505, lon: -46.6333, tz: -180 },
    CityPreset { name: "Rio de Janeiro, BR", lat: -22.9068, lon: -43.1729, tz: -180 },
    CityPreset { name: "Brasília, BR", lat: -15.8267, lon: -47.9218, tz: -180 },
    CityPreset { name: "Salvador, BR", lat: -12.9777, lon: -38.5016, tz: -180 },
    CityPreset { name: "Madrid, ES", lat: 40.4168, lon: -3.7038, tz: 60 },
    CityPreset { name: "Barcelona, ES", lat: 41.3851, lon: 2.1734, tz: 60 },
    CityPreset { name: "Sevilla, ES", lat: 37.3891, lon: -5.9845, tz: 60 },
    CityPreset { name: "Valencia, ES", lat: 39.4699, lon: -0.3763, tz: 60 },
    CityPreset { name: "Bilbao, ES", lat: 43.2630, lon: -2.9350, tz: 60 },
    CityPreset { name: "London, UK", lat: 51.5074, lon: -0.1278, tz: 0 },
    CityPreset { name: "Paris, FR", lat: 48.8566, lon: 2.3522, tz: 60 },
    CityPreset { name: "Berlin, DE", lat: 52.5200, lon: 13.4050, tz: 60 },
    CityPreset { name: "München, DE", lat: 48.1351, lon: 11.5820, tz: 60 },
    CityPreset { name: "Roma, IT", lat: 41.9028, lon: 12.4964, tz: 60 },
    CityPreset { name: "Milano, IT", lat: 45.4642, lon: 9.1900, tz: 60 },
    CityPreset { name: "Amsterdam, NL", lat: 52.3676, lon: 4.9041, tz: 60 },
    CityPreset { name: "Bruxelles, BE", lat: 50.8503, lon: 4.3517, tz: 60 },
    CityPreset { name: "Wien, AT", lat: 48.2082, lon: 16.3738, tz: 60 },
    CityPreset { name: "Zürich, CH", lat: 47.3769, lon: 8.5417, tz: 60 },
    CityPreset { name: "Lisboa, PT", lat: 38.7223, lon: -9.1393, tz: 0 },
    CityPreset { name: "Dublin, IE", lat: 53.3498, lon: -6.2603, tz: 0 },
    CityPreset { name: "Stockholm, SE", lat: 59.3293, lon: 18.0686, tz: 60 },
    CityPreset { name: "Oslo, NO", lat: 59.9139, lon: 10.7522, tz: 60 },
    CityPreset { name: "København, DK", lat: 55.6761, lon: 12.5683, tz: 60 },
    CityPreset { name: "Helsinki, FI", lat: 60.1699, lon: 24.9384, tz: 120 },
    CityPreset { name: "Warszawa, PL", lat: 52.2297, lon: 21.0122, tz: 60 },
    CityPreset { name: "Praha, CZ", lat: 50.0755, lon: 14.4378, tz: 60 },
    CityPreset { name: "Budapest, HU", lat: 47.4979, lon: 19.0402, tz: 60 },
    CityPreset { name: "Athina, GR", lat: 37.9838, lon: 23.7275, tz: 120 },
    CityPreset { name: "İstanbul, TR", lat: 41.0082, lon: 28.9784, tz: 180 },
    CityPreset { name: "Moskva, RU", lat: 55.7558, lon: 37.6173, tz: 180 },
    CityPreset { name: "New York, US", lat: 40.7128, lon: -74.0060, tz: -300 },
    CityPreset { name: "Los Angeles, US", lat: 34.0522, lon: -118.2437, tz: -480 },
    CityPreset { name: "Chicago, US", lat: 41.8781, lon: -87.6298, tz: -360 },
    CityPreset { name: "Miami, US", lat: 25.7617, lon: -80.1918, tz: -300 },
    CityPreset { name: "Houston, US", lat: 29.7604, lon: -95.3698, tz: -360 },
    CityPreset { name: "San Francisco, US", lat: 37.7749, lon: -122.4194, tz: -480 },
    CityPreset { name: "Seattle, US", lat: 47.6062, lon: -122.3321, tz: -480 },
    CityPreset { name: "Boston, US", lat: 42.3601, lon: -71.0589, tz: -300 },
    CityPreset { name: "Washington DC", lat: 38.9072, lon: -77.0369, tz: -300 },
    CityPreset { name: "Toronto, CA", lat: 43.6532, lon: -79.3832, tz: -300 },
    CityPreset { name: "Montreal, CA", lat: 45.5017, lon: -73.5673, tz: -300 },
    CityPreset { name: "Vancouver, CA", lat: 49.2827, lon: -123.1207, tz: -480 },
    CityPreset { name: "Tokyo, JP", lat: 35.6762, lon: 139.6503, tz: 540 },
    CityPreset { name: "Beijing, CN", lat: 39.9042, lon: 116.4074, tz: 480 },
    CityPreset { name: "Shanghai, CN", lat: 31.2304, lon: 121.4737, tz: 480 },
    CityPreset { name: "Hong Kong", lat: 22.3193, lon: 114.1694, tz: 480 },
    CityPreset { name: "Singapore", lat: 1.3521, lon: 103.8198, tz: 480 },
    CityPreset { name: "Seoul, KR", lat: 37.5665, lon: 126.9780, tz: 540 },
    CityPreset { name: "Bangkok, TH", lat: 13.7563, lon: 100.5018, tz: 420 },
    CityPreset { name: "Jakarta, ID", lat: -6.2088, lon: 106.8456, tz: 420 },
    CityPreset { name: "Manila, PH", lat: 14.5995, lon: 120.9842, tz: 480 },
    CityPreset { name: "Mumbai, IN", lat: 19.0760, lon: 72.8777, tz: 330 },
    CityPreset { name: "Delhi, IN", lat: 28.7041, lon: 77.1025, tz: 330 },
    CityPreset { name: "Bangalore, IN", lat: 12.9716, lon: 77.5946, tz: 330 },
    CityPreset { name: "Karachi, PK", lat: 24.8607, lon: 67.0011, tz: 300 },
    CityPreset { name: "Tehran, IR", lat: 35.6892, lon: 51.3890, tz: 210 },
    CityPreset { name: "Dubai, AE", lat: 25.2048, lon: 55.2708, tz: 240 },
    CityPreset { name: "Tel Aviv, IL", lat: 32.0853, lon: 34.7818, tz: 120 },
    CityPreset { name: "Cairo, EG", lat: 30.0444, lon: 31.2357, tz: 120 },
    CityPreset { name: "Lagos, NG", lat: 6.5244, lon: 3.3792, tz: 60 },
    CityPreset { name: "Nairobi, KE", lat: -1.2921, lon: 36.8219, tz: 180 },
    CityPreset { name: "Johannesburg, ZA", lat: -26.2041, lon: 28.0473, tz: 120 },
    CityPreset { name: "Cape Town, ZA", lat: -33.9249, lon: 18.4241, tz: 120 },
    CityPreset { name: "Casablanca, MA", lat: 33.5731, lon: -7.5898, tz: 60 },
    CityPreset { name: "Sydney, AU", lat: -33.8688, lon: 151.2093, tz: 600 },
    CityPreset { name: "Melbourne, AU", lat: -37.8136, lon: 144.9631, tz: 600 },
    CityPreset { name: "Auckland, NZ", lat: -36.8485, lon: 174.7633, tz: 720 },
    // --- Ampliación del atlas ---
    // Más Argentina / Cono Sur
    CityPreset { name: "La Plata, AR", lat: -34.9215, lon: -57.9545, tz: -180 },
    CityPreset { name: "Mar del Plata, AR", lat: -38.0055, lon: -57.5426, tz: -180 },
    CityPreset { name: "San Miguel de Tucumán, AR", lat: -26.8083, lon: -65.2176, tz: -180 },
    CityPreset { name: "Salta, AR", lat: -24.7821, lon: -65.4232, tz: -180 },
    CityPreset { name: "Neuquén, AR", lat: -38.9516, lon: -68.0591, tz: -180 },
    CityPreset { name: "Bariloche, AR", lat: -41.1335, lon: -71.3103, tz: -180 },
    CityPreset { name: "Ushuaia, AR", lat: -54.8019, lon: -68.3030, tz: -180 },
    CityPreset { name: "Concepción, CL", lat: -36.8201, lon: -73.0444, tz: -240 },
    CityPreset { name: "Antofagasta, CL", lat: -23.6509, lon: -70.3975, tz: -240 },
    // Más América Latina
    CityPreset { name: "Arequipa, PE", lat: -16.4090, lon: -71.5375, tz: -300 },
    CityPreset { name: "Trujillo, PE", lat: -8.1159, lon: -79.0300, tz: -300 },
    CityPreset { name: "Barranquilla, CO", lat: 10.9685, lon: -74.7813, tz: -300 },
    CityPreset { name: "Cartagena, CO", lat: 10.3910, lon: -75.4794, tz: -300 },
    CityPreset { name: "Santa Cruz, BO", lat: -17.7833, lon: -63.1821, tz: -240 },
    CityPreset { name: "Cochabamba, BO", lat: -17.3895, lon: -66.1568, tz: -240 },
    CityPreset { name: "Tijuana, MX", lat: 32.5149, lon: -117.0382, tz: -480 },
    CityPreset { name: "Cancún, MX", lat: 21.1619, lon: -86.8515, tz: -300 },
    CityPreset { name: "Puebla, MX", lat: 19.0414, lon: -98.2063, tz: -360 },
    CityPreset { name: "Belo Horizonte, BR", lat: -19.9167, lon: -43.9345, tz: -180 },
    CityPreset { name: "Porto Alegre, BR", lat: -30.0346, lon: -51.2177, tz: -180 },
    CityPreset { name: "Fortaleza, BR", lat: -3.7319, lon: -38.5267, tz: -180 },
    CityPreset { name: "Recife, BR", lat: -8.0476, lon: -34.8770, tz: -180 },
    CityPreset { name: "Curitiba, BR", lat: -25.4284, lon: -49.2733, tz: -180 },
    CityPreset { name: "Manaus, BR", lat: -3.1190, lon: -60.0217, tz: -240 },
    // Más Europa
    CityPreset { name: "Málaga, ES", lat: 36.7213, lon: -4.4214, tz: 60 },
    CityPreset { name: "Zaragoza, ES", lat: 41.6488, lon: -0.8891, tz: 60 },
    CityPreset { name: "Palma de Mallorca, ES", lat: 39.5696, lon: 2.6502, tz: 60 },
    CityPreset { name: "Las Palmas, ES", lat: 28.1235, lon: -15.4363, tz: 0 },
    CityPreset { name: "Porto, PT", lat: 41.1579, lon: -8.6291, tz: 0 },
    CityPreset { name: "Marseille, FR", lat: 43.2965, lon: 5.3698, tz: 60 },
    CityPreset { name: "Lyon, FR", lat: 45.7640, lon: 4.8357, tz: 60 },
    CityPreset { name: "Hamburg, DE", lat: 53.5511, lon: 9.9937, tz: 60 },
    CityPreset { name: "Frankfurt, DE", lat: 50.1109, lon: 8.6821, tz: 60 },
    CityPreset { name: "Napoli, IT", lat: 40.8518, lon: 14.2681, tz: 60 },
    CityPreset { name: "Manchester, UK", lat: 53.4808, lon: -2.2426, tz: 0 },
    CityPreset { name: "Edinburgh, UK", lat: 55.9533, lon: -3.1883, tz: 0 },
    CityPreset { name: "Kyiv, UA", lat: 50.4501, lon: 30.5234, tz: 120 },
    CityPreset { name: "Bucureşti, RO", lat: 44.4268, lon: 26.1025, tz: 120 },
    CityPreset { name: "Beograd, RS", lat: 44.7866, lon: 20.4489, tz: 60 },
    CityPreset { name: "Sankt-Peterburg, RU", lat: 59.9311, lon: 30.3609, tz: 180 },
    // Más Asia / Medio Oriente / África / Oceanía
    CityPreset { name: "Osaka, JP", lat: 34.6937, lon: 135.5023, tz: 540 },
    CityPreset { name: "Guangzhou, CN", lat: 23.1291, lon: 113.2644, tz: 480 },
    CityPreset { name: "Taipei, TW", lat: 25.0330, lon: 121.5654, tz: 480 },
    CityPreset { name: "Kuala Lumpur, MY", lat: 3.1390, lon: 101.6869, tz: 480 },
    CityPreset { name: "Ho Chi Minh, VN", lat: 10.8231, lon: 106.6297, tz: 420 },
    CityPreset { name: "Hanoi, VN", lat: 21.0278, lon: 105.8342, tz: 420 },
    CityPreset { name: "Dhaka, BD", lat: 23.8103, lon: 90.4125, tz: 360 },
    CityPreset { name: "Kolkata, IN", lat: 22.5726, lon: 88.3639, tz: 330 },
    CityPreset { name: "Chennai, IN", lat: 13.0827, lon: 80.2707, tz: 330 },
    CityPreset { name: "Hyderabad, IN", lat: 17.3850, lon: 78.4867, tz: 330 },
    CityPreset { name: "Riyadh, SA", lat: 24.7136, lon: 46.6753, tz: 180 },
    CityPreset { name: "Doha, QA", lat: 25.2854, lon: 51.5310, tz: 180 },
    CityPreset { name: "Baghdad, IQ", lat: 33.3152, lon: 44.3661, tz: 180 },
    CityPreset { name: "Amman, JO", lat: 31.9454, lon: 35.9284, tz: 120 },
    CityPreset { name: "Beirut, LB", lat: 33.8938, lon: 35.5018, tz: 120 },
    CityPreset { name: "Jerusalem, IL", lat: 31.7683, lon: 35.2137, tz: 120 },
    CityPreset { name: "Addis Abeba, ET", lat: 9.0300, lon: 38.7400, tz: 180 },
    CityPreset { name: "Accra, GH", lat: 5.6037, lon: -0.1870, tz: 0 },
    CityPreset { name: "Dakar, SN", lat: 14.7167, lon: -17.4677, tz: 0 },
    CityPreset { name: "Alger, DZ", lat: 36.7538, lon: 3.0588, tz: 60 },
    CityPreset { name: "Tunis, TN", lat: 36.8065, lon: 10.1815, tz: 60 },
    CityPreset { name: "Brisbane, AU", lat: -27.4698, lon: 153.0251, tz: 600 },
    CityPreset { name: "Perth, AU", lat: -31.9505, lon: 115.8605, tz: 480 },
    CityPreset { name: "Wellington, NZ", lat: -41.2865, lon: 174.7762, tz: 720 },
];
