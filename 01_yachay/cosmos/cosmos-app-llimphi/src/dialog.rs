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
    prelude::{auto, length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_calendar::{calendar_view, CalendarPalette, CalendarSpec, WeekStart};
use llimphi_widget_panel::{panel_signature_painter, PanelStyle};
use llimphi_widget_text_input::{text_input_view_mouse, TextInputPalette};

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

pub(crate) use cosmos_cities::City;

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
    /// Offset UTC en minutos para la fecha de la carta — computado de la
    /// zona IANA al elegir ciudad (con el DST de la época) y recomputado al
    /// confirmar contra la fecha final.
    pub tz: i32,
    /// Zona horaria IANA de la ciudad elegida (`""` hasta elegir una). Es la
    /// fuente de verdad del offset histórico; ver `cosmos-cities`.
    pub tz_iana: String,
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
            (Dialog::NewChart(c), DialogField::Date) => {
                c.date = v;
                // Escribir la fecha (incluido el año) mueve el calendario
                // inline a ese mes/año — así se navega a años lejanos sin
                // clickar el stepper mil veces.
                if let Some(d) = parse_naive_date(&c.date) {
                    use chrono::Datelike;
                    c.cal_year = d.year();
                    c.cal_month = d.month();
                }
            }
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

/// Cuántas ciudades ofrece el picker. Render y handler de pick usan el
/// mismo número para que el índice clickeado coincida.
pub(crate) const CITY_LIMIT: usize = 8;

/// Ciudades del atlas que matchean la query (offline, fuzzy, sin tildes,
/// rankeadas por población). Ver `cosmos-cities`.
pub(crate) fn city_matches(query: &str) -> Vec<&'static City> {
    cosmos_cities::search(query, CITY_LIMIT)
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
                // Acepta el nombre tecleado como contacto nuevo y avanza el
                // foco a Etiqueta (eso cierra el desplegable). El contacto se
                // crea de verdad al confirmar (contact == None + query).
                Msg::DialogFocus(DialogField::Label),
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
            for (idx, city) in city_matches(&c.city_query).into_iter().enumerate() {
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
                    .text_aligned(city.label(), 11.0, theme.fg_muted, Alignment::Start)]),
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
            for (idx, city) in city_matches(&c.city_query).into_iter().enumerate() {
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
                    .text_aligned(city.label(), 11.0, theme.fg_muted, Alignment::Start)]),
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
        text_input_view_mouse(
            &model.dialog_input,
            "",
            true,
            &TextInputPalette::from_theme(theme),
            move |x| Msg::DialogClickAt(field, x),
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
        .on_click_at(move |x, _y, _w, _h| Some(Msg::DialogClickAt(field, x)))
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
    // Navegación rápida de año (el header del calendar sólo mueve meses):
    // «  ‹  AÑO  ›  »  donde « = −10, ‹ = −1, › = +1, » = +10.
    let y = form.cal_year;
    let mo = form.cal_month;
    let yr_btn = |label: &str, delta: i32| -> View<Msg> {
        View::new(Style {
            size: Size {
                width: length(26.0_f32),
                height: length(22.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .radius(4.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::DialogCalView(y + delta, mo))
        .children(vec![View::new(Style {
            size: Size {
                width: auto(),
                height: auto(),
            },
            ..Default::default()
        })
        .text_aligned(label.to_string(), 11.0, theme.fg_text, Alignment::Center)])
    };
    let year_nav = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: auto(),
            height: length(22.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(4.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        yr_btn("«", -10),
        yr_btn("‹", -1),
        View::new(Style {
            size: Size {
                width: length(48.0_f32),
                height: auto(),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .text_aligned(format!("{y}"), 12.0, theme.accent, Alignment::Center),
        yr_btn("›", 1),
        yr_btn("»", 10),
    ]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(278.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .children(vec![year_nav, cal])
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

