//! El árbol de vistas de raymi: barra superior + cuerpo (calendario o
//! contactos) + barra de estado. Todo se reconstruye desde `&Model` (Elm puro).

use std::collections::HashMap;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, LengthPercentage, Position, Rect, Size, Style},
    style::LengthPercentageAuto,
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_text_input::{text_input_view, TextInputPalette};

use raymi_core::time::{self, CivilDate, DAY};
use raymi_core::{Address, CalStore, Contact, Occurrence};

use crate::{
    CalView, ContactDraft, ContactField, EventDraft, EventField, Mode, Model, Msg, Repeat, RepeatEnd,
};
use llimphi_widget_text_input::TextInputState;

const AGENDA_W: f32 = 340.0;
const CONTACTS_W: f32 = 340.0;
const TOOLBAR_H: f32 = 48.0;
const STATUS_H: f32 = 26.0;
const AVATAR: f32 = 38.0;

const MONTHS: [&str; 12] = [
    "Enero", "Febrero", "Marzo", "Abril", "Mayo", "Junio", "Julio", "Agosto", "Septiembre",
    "Octubre", "Noviembre", "Diciembre",
];
const WEEKDAYS: [&str; 7] = ["Lun", "Mar", "Mié", "Jue", "Vie", "Sáb", "Dom"];

fn pad_xy(x: f32, y: f32) -> Rect<LengthPercentage> {
    Rect { left: length(x), right: length(x), top: length(y), bottom: length(y) }
}

/// La raíz: barra superior, cuerpo según modo, barra de estado.
pub fn root(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let body = match model.mode {
        Mode::Calendar => calendar_body(model),
        Mode::Contacts => contacts_body(model),
    };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![toolbar(model), body, status_bar(model)])
}

/// Barra superior: marca + tabs de modo + (en calendario) navegación de mes.
fn toolbar(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let brand = View::new(Style {
        size: Size { width: length(120.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        padding: pad_xy(16.0, 0.0),
        ..Default::default()
    })
    .text_aligned("📅  raymi", 18.0, theme.fg_text, Alignment::Start);

    let tabs = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![
        tab(theme, "Calendario", model.mode == Mode::Calendar, Msg::SetMode(Mode::Calendar)),
        tab(theme, "Contactos", model.mode == Mode::Contacts, Msg::SetMode(Mode::Contacts)),
    ]);

    let mut children = vec![brand, tabs, spacer()];
    if model.mode == Mode::Calendar {
        // Conmutador Mes / Semana.
        children.push(view_tab(theme, "Mes", model.cal_view() == CalView::Month, Msg::SetCalView(CalView::Month)));
        children.push(view_tab(theme, "Semana", model.cal_view() == CalView::Week, Msg::SetCalView(CalView::Week)));
        children.push(button("＋ Evento", theme.accent, theme.bg_app, Msg::NewEvent));
        let label = match model.cal_view() {
            CalView::Month => format!("{}  {}", MONTHS[(model.view_month - 1) as usize], model.view_year),
            CalView::Week => week_label(model),
        };
        children.push(button("‹", theme.bg_button, theme.fg_text, Msg::PrevMonth));
        children.push(
            View::new(Style {
                size: Size { width: length(176.0_f32), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .text(label, 14.0, theme.fg_text),
        );
        children.push(button("›", theme.bg_button, theme.fg_text, Msg::NextMonth));
        children.push(button("Hoy", theme.accent, theme.bg_app, Msg::Today));
    } else {
        children.push(button("＋ Contacto", theme.accent, theme.bg_app, Msg::NewContact));
    }

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(TOOLBAR_H) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        padding: pad_xy(12.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(children)
}

fn tab(theme: &Theme, label: &str, active: bool, msg: Msg) -> View<Msg> {
    let (bg, fg) = if active { (theme.bg_selected, theme.fg_text) } else { (theme.bg_panel, theme.fg_muted) };
    View::new(Style {
        size: Size { width: Dimension::auto(), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: pad_xy(14.0, 0.0),
        ..Default::default()
    })
    .fill(bg)
    .radius(6.0)
    .hover_fill(theme.bg_row_hover)
    .text(label, 13.0, fg)
    .on_click(msg)
}

// ── Calendario ────────────────────────────────────────────────────────────

fn calendar_body(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let inner = match model.cal_view() {
        CalView::Month => vec![month_grid(model), day_agenda(model)],
        CalView::Week => vec![week_grid(model)],
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(inner)
    .fill(theme.bg_app)
}

/// Tab chico para el conmutador de vista (Mes / Semana).
fn view_tab(theme: &Theme, label: &str, active: bool, msg: Msg) -> View<Msg> {
    let (bg, fg) = if active { (theme.bg_selected, theme.fg_text) } else { (theme.bg_button, theme.fg_muted) };
    View::new(Style {
        size: Size { width: Dimension::auto(), height: length(28.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: pad_xy(12.0, 0.0),
        ..Default::default()
    })
    .fill(bg)
    .radius(6.0)
    .hover_fill(theme.bg_row_hover)
    .text(label, 12.5, fg)
    .on_click(msg)
}

/// Etiqueta del rango de la semana mostrada (lunes–domingo).
fn week_label(model: &Model) -> String {
    let days = model.selected_day.div_euclid(DAY);
    let monday = days - time::weekday(days) as i64;
    let a = time::civil_from_days(monday);
    let b = time::civil_from_days(monday + 6);
    if a.month == b.month {
        format!("{}–{} {} {}", a.day, b.day, MONTHS[(a.month - 1) as usize], a.year)
    } else {
        format!("{} {} – {} {}", a.day, &MONTHS[(a.month - 1) as usize][..3], b.day, &MONTHS[(b.month - 1) as usize][..3])
    }
}

/// Grilla del mes: cabecera de días + 6 semanas × 7 días con chips de eventos.
fn month_grid(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let first = CivilDate { year: model.view_year, month: model.view_month, day: 1 };
    let first_days = time::days_from_civil(first.year, first.month, first.day);
    let grid_start = first_days - time::weekday(first_days) as i64; // lunes en/antes del 1

    // Ocurrencias de toda la grilla, agrupadas por día.
    let occ = model.store_ref().occurrences_in(grid_start * DAY, (grid_start + 42) * DAY);
    let mut by_day: HashMap<i64, Vec<&Occurrence>> = HashMap::new();
    for o in &occ {
        by_day.entry(time::start_of_day(o.start)).or_default().push(o);
    }
    let colors = calendar_colors(model);

    // Cabecera de días de la semana.
    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(
        WEEKDAYS
            .iter()
            .map(|w| {
                View::new(Style {
                    size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
                    flex_grow: 1.0,
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::Center),
                    ..Default::default()
                })
                .text(*w, 11.0, theme.fg_muted)
            })
            .collect(),
    );

    // 6 semanas.
    let mut weeks: Vec<View<Msg>> = Vec::with_capacity(6);
    for wk in 0..6 {
        let mut cells: Vec<View<Msg>> = Vec::with_capacity(7);
        for d in 0..7 {
            let day_days = grid_start + wk * 7 + d;
            let day_ts = day_days * DAY;
            let date = time::civil_from_days(day_days);
            let in_month = date.month == model.view_month;
            let is_today = day_ts == model.today;
            let is_selected = day_ts == model.selected_day;
            let evs = by_day.get(&day_ts).cloned().unwrap_or_default();
            cells.push(day_cell(theme, date, in_month, is_today, is_selected, day_ts, &evs, &colors));
        }
        weeks.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size { width: percent(1.0_f32), height: Dimension::auto() },
                flex_grow: 1.0,
                gap: Size { width: length(1.0_f32), height: length(0.0_f32) },
                ..Default::default()
            })
            .children(cells),
        );
    }

    let grid = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(1.0_f32) },
        ..Default::default()
    })
    .children(weeks);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        padding: pad_xy(6.0, 6.0),
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        ..Default::default()
    })
    .children(vec![header, grid])
}

#[allow(clippy::too_many_arguments)]
fn day_cell(
    theme: &Theme,
    date: CivilDate,
    in_month: bool,
    is_today: bool,
    is_selected: bool,
    day_ts: i64,
    events: &[&Occurrence],
    colors: &HashMap<String, Color>,
) -> View<Msg> {
    let bg = if is_selected { theme.bg_selected } else if in_month { theme.bg_panel } else { theme.bg_panel_alt };
    let num_color = if !in_month {
        theme.fg_placeholder
    } else if is_today {
        theme.bg_app
    } else {
        theme.fg_text
    };

    // Número del día (con disco de acento si es hoy).
    let num = View::new(Style {
        size: Size { width: length(22.0_f32), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(if is_today { theme.accent } else { bg })
    .radius(10.0)
    .text(date.day.to_string(), 12.0, num_color);

    let head = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![num]);

    // Hasta 3 chips + "+N".
    let mut chips: Vec<View<Msg>> = Vec::new();
    for o in events.iter().take(3) {
        let color = colors.get(&o.event.calendar).copied().unwrap_or(theme.accent);
        chips.push(event_chip(theme, &o.event.summary, color));
    }
    if events.len() > 3 {
        chips.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(13.0_f32) },
                padding: pad_xy(4.0, 0.0),
                ..Default::default()
            })
            .text_aligned(format!("+{} más", events.len() - 3), 10.0, theme.fg_muted, Alignment::Start),
        );
    }
    let chip_col = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .clip(true)
    .children(chips);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        padding: pad_xy(4.0, 3.0),
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .fill(bg)
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::SelectDay(day_ts))
    .children(vec![head, chip_col])
}

fn event_chip(theme: &Theme, summary: &str, color: Color) -> View<Msg> {
    let bar = View::new(Style {
        size: Size { width: length(3.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(color)
    .radius(2.0);
    let label = View::new(Style {
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(summary.to_string(), 10.5, theme.fg_text, Alignment::Start);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(15.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
        padding: pad_xy(3.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(3.0)
    .clip(true)
    .children(vec![bar, label])
}

/// Agenda del día seleccionado: instancias con hora, color y asunto.
fn day_agenda(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let day = model.selected_day;
    let occ = model.store_ref().occurrences_in(day, day + DAY);
    let colors = calendar_colors(model);

    let date = time::civil_from_days(day.div_euclid(DAY));
    let wd = WEEKDAYS[time::weekday(day.div_euclid(DAY)) as usize];
    let header = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(50.0_f32) },
        justify_content: Some(JustifyContent::Center),
        padding: pad_xy(16.0, 0.0),
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![
        View::new(Style { size: Size { width: percent(1.0_f32), height: length(22.0_f32) }, ..Default::default() })
            .text_aligned(format!("{wd} {}", date.day), 18.0, theme.fg_text, Alignment::Start),
        View::new(Style { size: Size { width: percent(1.0_f32), height: length(14.0_f32) }, ..Default::default() })
            .text_aligned(
                format!("{} {}", MONTHS[(date.month - 1) as usize], date.year),
                11.0,
                theme.fg_muted,
                Alignment::Start,
            ),
    ]);

    let mut rows: Vec<View<Msg>> = Vec::new();
    for o in &occ {
        let color = colors.get(&o.event.calendar).copied().unwrap_or(theme.accent);
        rows.push(agenda_row(theme, o, color));
    }
    if rows.is_empty() {
        rows.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(60.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .text_aligned("sin eventos", 13.0, theme.fg_placeholder, Alignment::Center),
        );
    }
    let list = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        padding: pad_xy(10.0, 10.0),
        ..Default::default()
    })
    .clip(true)
    .children(rows);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(AGENDA_W), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![header, list])
}

fn agenda_row(theme: &Theme, o: &Occurrence, color: Color) -> View<Msg> {
    let mut when = if o.event.all_day {
        "todo el día".to_string()
    } else {
        format!("{} – {}", hhmm(o.start), hhmm(o.end))
    };
    if !o.event.attendees.is_empty() {
        when.push_str(&format!("  ·  👤 {}", o.event.attendees.len()));
    }
    let bar = View::new(Style {
        size: Size { width: length(4.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(color)
    .radius(2.0);

    let texts = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        justify_content: Some(JustifyContent::Center),
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .children(vec![
        View::new(Style { size: Size { width: percent(1.0_f32), height: length(16.0_f32) }, ..Default::default() })
            .text_aligned(o.event.summary.clone(), 13.0, theme.fg_text, Alignment::Start),
        View::new(Style { size: Size { width: percent(1.0_f32), height: length(14.0_f32) }, ..Default::default() })
            .text_aligned(
                if o.event.location.is_empty() { when.clone() } else { format!("{when}  ·  {}", o.event.location) },
                11.0,
                theme.fg_muted,
                Alignment::Start,
            ),
    ]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(44.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        padding: pad_xy(10.0, 6.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(6.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::EditEvent {
        calendar: o.event.calendar.clone(),
        uid: o.event.uid.clone(),
        occ_start: Some(o.start),
    })
    .children(vec![bar, texts])
}

// ── Vista semana (rejilla horaria) ───────────────────────────────────────────

const WEEK_START_H: i64 = 7;
const WEEK_END_H: i64 = 22;
const WK_HOURS: i64 = WEEK_END_H - WEEK_START_H;
const HOUR_PX: f32 = 36.0;
const WK_GRID_H: f32 = WK_HOURS as f32 * HOUR_PX;
const GUTTER_W: f32 = 52.0;
const WK_HEADER_H: f32 = 46.0;
const ALLDAY_H: f32 = 28.0;

/// La semana (lunes–domingo) del día seleccionado: cabecera + franja de día
/// completo + rejilla horaria con bloques posicionados por hora.
fn week_grid(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let days0 = model.selected_day.div_euclid(DAY);
    let monday = days0 - time::weekday(days0) as i64;
    let monday_ts = monday * DAY;
    let colors = calendar_colors(model);

    // Ocurrencias de la semana, repartidas por día de su inicio.
    let occ = model.store_ref().occurrences_in(monday_ts, monday_ts + 7 * DAY);
    let mut by_day: Vec<Vec<&Occurrence>> = vec![Vec::new(); 7];
    for o in &occ {
        let idx = (time::start_of_day(o.start) - monday_ts).div_euclid(DAY);
        if (0..7).contains(&idx) {
            by_day[idx as usize].push(o);
        }
    }

    // Cabecera: hueco del medidor + 7 días.
    let mut head = vec![gutter_box(WK_HEADER_H)];
    for i in 0..7 {
        let day_ts = (monday + i) * DAY;
        let date = time::civil_from_days(monday + i);
        head.push(wk_day_header(theme, date, day_ts, day_ts == model.today, day_ts == model.selected_day));
    }
    let header = row_full(WK_HEADER_H, head);

    // Franja de eventos de día completo.
    let mut strip = vec![gutter_label(theme, "todo el día", ALLDAY_H)];
    for day in by_day.iter() {
        let chips: Vec<View<Msg>> = day
            .iter()
            .filter(|o| o.event.all_day)
            .map(|o| {
                let color = colors.get(&o.event.calendar).copied().unwrap_or(theme.accent);
                wk_allday_chip(theme, o, color)
            })
            .collect();
        strip.push(
            View::new(Style {
                size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
                flex_grow: 1.0,
                align_items: Some(AlignItems::Center),
                gap: Size { width: length(3.0_f32), height: length(0.0_f32) },
                padding: pad_xy(3.0, 0.0),
                ..Default::default()
            })
            .clip(true)
            .children(chips),
        );
    }
    let allday = row_full(ALLDAY_H, strip);

    // Rejilla horaria: medidor + 7 columnas.
    let mut grid = vec![time_gutter(theme)];
    for (i, day) in by_day.into_iter().enumerate() {
        let day_ts = (monday + i as i64) * DAY;
        grid.push(wk_day_column(theme, day, day_ts, &colors));
    }
    let body = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(WK_GRID_H) },
        ..Default::default()
    })
    .children(grid);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![header, allday, body])
}

/// Fila a lo ancho con altura fija (cabecera / franja).
fn row_full(h: f32, children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(h) },
        gap: Size { width: length(1.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(children)
}

fn gutter_box(h: f32) -> View<Msg> {
    View::new(Style { size: Size { width: length(GUTTER_W), height: length(h) }, ..Default::default() })
}

fn gutter_label(theme: &Theme, text: &str, h: f32) -> View<Msg> {
    View::new(Style {
        size: Size { width: length(GUTTER_W), height: length(h) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(text.to_string(), 9.5, theme.fg_placeholder, Alignment::Center)
}

fn wk_day_header(theme: &Theme, date: CivilDate, day_ts: i64, is_today: bool, is_selected: bool) -> View<Msg> {
    let wd = WEEKDAYS[time::weekday(day_ts.div_euclid(DAY)) as usize];
    let bg = if is_selected { theme.bg_selected } else { theme.bg_panel };
    let num_bg = if is_today { theme.accent } else { bg };
    let num_fg = if is_today { theme.bg_app } else { theme.fg_text };
    let num = View::new(Style {
        size: Size { width: length(26.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(num_bg)
    .radius(11.0)
    .text(date.day.to_string(), 14.0, num_fg);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::SelectDay(day_ts))
    .children(vec![
        View::new(Style { size: Size { width: percent(1.0_f32), height: length(12.0_f32) }, align_items: Some(AlignItems::Center), justify_content: Some(JustifyContent::Center), ..Default::default() })
            .text(wd.to_string(), 10.0, theme.fg_muted),
        num,
    ])
}

fn wk_allday_chip(theme: &Theme, o: &Occurrence, color: Color) -> View<Msg> {
    let bar = View::new(Style { size: Size { width: length(3.0_f32), height: percent(1.0_f32) }, ..Default::default() })
        .fill(color)
        .radius(2.0);
    let label = View::new(Style { size: Size { width: Dimension::auto(), height: percent(1.0_f32) }, flex_grow: 1.0, align_items: Some(AlignItems::Center), ..Default::default() })
        .text_aligned(o.event.summary.clone(), 10.0, theme.fg_text, Alignment::Start);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(3.0_f32), height: length(0.0_f32) },
        padding: pad_xy(3.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(3.0)
    .clip(true)
    .on_click(Msg::EditEvent { calendar: o.event.calendar.clone(), uid: o.event.uid.clone(), occ_start: Some(o.start) })
    .children(vec![bar, label])
}

/// Medidor de horas a la izquierda: etiquetas absolutas en cada línea horaria.
fn time_gutter(theme: &Theme) -> View<Msg> {
    let mut labels: Vec<View<Msg>> = Vec::new();
    for h in 0..WK_HOURS {
        labels.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(0.0_f32),
                    right: length(4.0_f32),
                    top: length(h as f32 * HOUR_PX - 6.0),
                    bottom: LengthPercentageAuto::auto(),
                },
                size: Size { width: Dimension::auto(), height: length(12.0_f32) },
                ..Default::default()
            })
            .text_aligned(format!("{:02}:00", WEEK_START_H + h), 9.5, theme.fg_placeholder, Alignment::End),
        );
    }
    View::new(Style {
        size: Size { width: length(GUTTER_W), height: length(WK_GRID_H) },
        ..Default::default()
    })
    .children(labels)
}

/// Una columna-día: líneas horarias de fondo + bloques de eventos posicionados.
fn wk_day_column(theme: &Theme, day: Vec<&Occurrence>, day_ts: i64, colors: &HashMap<String, Color>) -> View<Msg> {
    let col_start = day_ts + WEEK_START_H * 3600;
    let mut children: Vec<View<Msg>> = Vec::new();

    // Líneas horarias.
    for h in 0..=WK_HOURS {
        children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect { left: length(0.0_f32), right: length(0.0_f32), top: length(h as f32 * HOUR_PX), bottom: LengthPercentageAuto::auto() },
                size: Size { width: Dimension::auto(), height: length(1.0_f32) },
                ..Default::default()
            })
            .fill(theme.border),
        );
    }

    // Bloques de eventos (sólo con hora; los de día completo van a la franja).
    for o in day.iter().filter(|o| !o.event.all_day) {
        let color = colors.get(&o.event.calendar).copied().unwrap_or(theme.accent);
        let top = (((o.start - col_start) as f32) / 3600.0 * HOUR_PX).clamp(0.0, WK_GRID_H - 14.0);
        let bottom = (((o.end - col_start) as f32) / 3600.0 * HOUR_PX).clamp(0.0, WK_GRID_H);
        let h = (bottom - top).max(15.0);
        children.push(wk_event_block(theme, o, color, top, h));
    }

    View::new(Style {
        size: Size { width: Dimension::auto(), height: length(WK_GRID_H) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(children)
}

fn wk_event_block(theme: &Theme, o: &Occurrence, color: Color, top: f32, h: f32) -> View<Msg> {
    let bar = View::new(Style { size: Size { width: length(3.0_f32), height: percent(1.0_f32) }, ..Default::default() })
        .fill(color)
        .radius(2.0);
    let summary = View::new(Style { size: Size { width: percent(1.0_f32), height: length(13.0_f32) }, ..Default::default() })
        .text_aligned(o.event.summary.clone(), 10.5, theme.fg_text, Alignment::Start);
    let mut texts = vec![summary];
    if h >= 30.0 {
        texts.push(
            View::new(Style { size: Size { width: percent(1.0_f32), height: length(11.0_f32) }, ..Default::default() })
                .text_aligned(format!("{} – {}", hhmm(o.start), hhmm(o.end)), 9.0, theme.fg_muted, Alignment::Start),
        );
    }
    let col = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(1.0_f32) },
        ..Default::default()
    })
    .clip(true)
    .children(texts);
    View::new(Style {
        position: Position::Absolute,
        inset: Rect { left: length(2.0_f32), right: length(2.0_f32), top: length(top), bottom: LengthPercentageAuto::auto() },
        size: Size { width: Dimension::auto(), height: length(h) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Stretch),
        gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
        padding: pad_xy(4.0, 2.0),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::EditEvent { calendar: o.event.calendar.clone(), uid: o.event.uid.clone(), occ_start: Some(o.start) })
    .children(vec![bar, col])
}

// ── Contactos ───────────────────────────────────────────────────────────────

fn contacts_body(model: &Model) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![contacts_list(model), contact_detail(model)])
}

fn contacts_list(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let pal = TextInputPalette::from_theme(theme);
    let query = model.search.text();
    let hits = model.store_ref().search_contacts(&query);

    let search = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(46.0_f32) },
        align_items: Some(AlignItems::Center),
        padding: pad_xy(10.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![text_input_view(
        &model.search,
        "🔍  Buscar contacto…",
        model.search_focused,
        &pal,
        Msg::ContactSearchFocus(true),
    )]);

    let mut rows: Vec<View<Msg>> = Vec::new();
    for c in &hits {
        let selected = model.selected_contact_uid() == Some(c.uid.as_str());
        rows.push(contact_row(theme, c, selected));
    }
    if rows.is_empty() {
        rows.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(50.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .text_aligned("sin contactos", 13.0, theme.fg_placeholder, Alignment::Center),
        );
    }
    let list = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        flex_grow: 1.0,
        ..Default::default()
    })
    .clip(true)
    .children(rows);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(CONTACTS_W), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![search, list])
}

fn contact_row(theme: &Theme, c: &Contact, selected: bool) -> View<Msg> {
    let bg = if selected { theme.bg_selected } else { theme.bg_panel };
    let name = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .text_aligned(c.full_name.clone(), 13.5, theme.fg_text, Alignment::Start);
    let mail = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(14.0_f32) },
        ..Default::default()
    })
    .text_aligned(c.primary_email().unwrap_or("").to_string(), 11.0, theme.fg_muted, Alignment::Start);
    let texts = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        justify_content: Some(JustifyContent::Center),
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .children(vec![name, mail]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(52.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(10.0_f32), height: length(0.0_f32) },
        padding: pad_xy(12.0, 6.0),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::SelectContact(c.uid.clone()))
    .children(vec![avatar(&c.initials(), &c.full_name), texts])
}

fn contact_detail(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let contact = model
        .selected_contact_uid()
        .and_then(|uid| model.store_ref().search_contacts("").into_iter().find(|c| c.uid == uid));

    let Some(c) = contact else {
        let ph = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned("Elegí un contacto", 14.0, theme.fg_placeholder, Alignment::Center);
        return View::new(Style {
            size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![ph]);
    };

    let head = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(72.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(14.0_f32), height: length(0.0_f32) },
        padding: pad_xy(20.0, 0.0),
        ..Default::default()
    })
    .children(vec![
        avatar_big(&c.initials(), &c.full_name),
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: Dimension::auto(), height: Dimension::auto() },
            flex_grow: 1.0,
            justify_content: Some(JustifyContent::Center),
            gap: Size { width: length(0.0_f32), height: length(3.0_f32) },
            ..Default::default()
        })
        .children(vec![
            View::new(Style { size: Size { width: percent(1.0_f32), height: length(24.0_f32) }, ..Default::default() })
                .text_aligned(c.full_name.clone(), 19.0, theme.fg_text, Alignment::Start),
            View::new(Style { size: Size { width: percent(1.0_f32), height: length(15.0_f32) }, ..Default::default() })
                .text_aligned(c.org.clone().unwrap_or_default(), 12.0, theme.fg_muted, Alignment::Start),
        ]),
        button("✎ Editar", theme.bg_button, theme.fg_text, Msg::EditContact(c.uid.clone())),
    ]);

    let mut fields: Vec<View<Msg>> = Vec::new();
    for e in &c.emails {
        fields.push(detail_field(theme, "✉", e));
    }
    for p in &c.phones {
        fields.push(detail_field(theme, "📞", p));
    }
    if !c.note.trim().is_empty() {
        fields.push(detail_field(theme, "📝", &c.note));
    }
    let body = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        padding: pad_xy(20.0, 12.0),
        ..Default::default()
    })
    .clip(true)
    .children(fields);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![head, body])
}

fn detail_field(theme: &Theme, icon: &str, value: &str) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(10.0_f32), height: length(0.0_f32) },
        padding: pad_xy(12.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(6.0)
    .children(vec![
        View::new(Style { size: Size { width: length(20.0_f32), height: percent(1.0_f32) }, align_items: Some(AlignItems::Center), justify_content: Some(JustifyContent::Center), ..Default::default() })
            .text(icon, 14.0, theme.fg_muted),
        View::new(Style { size: Size { width: Dimension::auto(), height: percent(1.0_f32) }, flex_grow: 1.0, align_items: Some(AlignItems::Center), ..Default::default() })
            .text_aligned(value.to_string(), 13.0, theme.fg_text, Alignment::Start),
    ])
}

// ── editores (overlay modal) ─────────────────────────────────────────────────

const EDITOR_W: f32 = 520.0;

/// Modal del editor de **evento**: selector de calendario, asunto, día completo,
/// fecha, horas, lugar y descripción, con acciones abajo.
pub fn event_editor(model: &Model, d: &EventDraft) -> View<Msg> {
    let theme = &model.theme;
    let pal = TextInputPalette::from_theme(theme);

    let title = if d.uid.is_some() { "Editar evento" } else { "Nuevo evento" };

    // Selector de calendario (clic → siguiente). Muestra punto de color + nombre.
    let colors = calendar_colors(model);
    let cal_name = model
        .store_ref()
        .calendars()
        .iter()
        .find(|c| c.id == d.calendar)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| d.calendar.clone());
    let cal_color = colors.get(&d.calendar).copied().unwrap_or(theme.accent);
    let cal_dot = View::new(Style {
        size: Size { width: length(12.0_f32), height: length(12.0_f32) },
        ..Default::default()
    })
    .fill(cal_color)
    .radius(6.0);
    let cal_chip = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        padding: pad_xy(10.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_input)
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::EventCycleCalendar)
    .children(vec![
        cal_dot,
        View::new(Style { size: Size { width: Dimension::auto(), height: percent(1.0_f32) }, flex_grow: 1.0, align_items: Some(AlignItems::Center), ..Default::default() })
            .text_aligned(cal_name, 13.0, theme.fg_text, Alignment::Start),
        View::new(Style { size: Size { width: length(80.0_f32), height: percent(1.0_f32) }, align_items: Some(AlignItems::Center), ..Default::default() })
            .text_aligned("cambiar ⟳".to_string(), 11.0, theme.fg_muted, Alignment::End),
    ]);

    let summary = ev_field(&d.summary, "Asunto", d.focus == EventField::Summary, &pal, EventField::Summary);
    let allday = checkbox(theme, "Día completo", d.all_day, Msg::EventToggleAllDay);
    let date = ev_field(&d.date, "AAAA-MM-DD", d.focus == EventField::Date, &pal, EventField::Date);

    let mut col: Vec<View<Msg>> = Vec::new();
    // Selector de alcance — sólo al editar una instancia de un recurrente.
    if d.is_recurring_instance() {
        col.push(labeled(
            theme,
            "Aplicar a",
            cycle_chip(theme, model.edit_scope().label(), Msg::EventCycleScope),
        ));
    }
    col.push(labeled(theme, "Calendario", cal_chip));
    col.push(labeled(theme, "Asunto", summary));
    col.push(allday);
    col.push(labeled(theme, "Fecha", date));
    if !d.all_day {
        let start = ev_field(&d.start_hm, "HH:MM", d.focus == EventField::Start, &pal, EventField::Start);
        let end = ev_field(&d.end_hm, "HH:MM", d.focus == EventField::End, &pal, EventField::End);
        let hours = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: Dimension::auto() },
            gap: Size { width: length(10.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .children(vec![labeled(theme, "Inicio", start), labeled(theme, "Fin", end)]);
        col.push(hours);
    }
    // Sección de repetición (cadencia + intervalo + días + término).
    for v in repeat_section(theme, &pal, d) {
        col.push(v);
    }
    col.push(labeled(
        theme,
        "Lugar",
        ev_field(&d.location, "Lugar (opcional)", d.focus == EventField::Location, &pal, EventField::Location),
    ));
    col.push(labeled(
        theme,
        "Descripción",
        ev_field(&d.description, "Notas (opcional)", d.focus == EventField::Description, &pal, EventField::Description),
    ));
    col.push(attendees_section(model, &pal, d));

    let actions = editor_actions(theme, d.uid.is_some(), Msg::SaveEvent, Msg::DeleteEvent);
    col.push(actions);

    editor_card(theme, title, col)
}

/// Modal del editor de **contacto**.
pub fn contact_editor(model: &Model, d: &ContactDraft) -> View<Msg> {
    let theme = &model.theme;
    let pal = TextInputPalette::from_theme(theme);
    let title = if d.uid.is_some() { "Editar contacto" } else { "Nuevo contacto" };

    let col: Vec<View<Msg>> = vec![
        labeled(theme, "Nombre", ct_field(&d.name, "Nombre y apellido", d.focus == ContactField::Name, &pal, ContactField::Name)),
        labeled(theme, "Correos", ct_field(&d.emails, "correo@dominio, otro@…", d.focus == ContactField::Emails, &pal, ContactField::Emails)),
        labeled(theme, "Teléfonos", ct_field(&d.phones, "+58 412…, …", d.focus == ContactField::Phones, &pal, ContactField::Phones)),
        labeled(theme, "Organización", ct_field(&d.org, "Empresa (opcional)", d.focus == ContactField::Org, &pal, ContactField::Org)),
        labeled(theme, "Nota", ct_field(&d.note, "Nota (opcional)", d.focus == ContactField::Note, &pal, ContactField::Note)),
        editor_actions(theme, d.uid.is_some(), Msg::SaveContact, Msg::DeleteContact),
    ];

    editor_card(theme, title, col)
}

const WD_INITIALS: [&str; 7] = ["L", "M", "X", "J", "V", "S", "D"];

/// Controles de repetición del editor de evento. Devuelve las filas a apilar:
/// siempre el selector de cadencia; si repite, intervalo, (días si es semanal) y
/// la condición de término con su campo contextual.
fn repeat_section(theme: &Theme, pal: &TextInputPalette, d: &EventDraft) -> Vec<View<Msg>> {
    let mut out = vec![labeled(
        theme,
        "Repetir",
        cycle_chip(theme, d.repeat.label(), Msg::EventCycleRepeat),
    )];
    if d.repeat == Repeat::None {
        return out;
    }

    // "cada [N] unidad"
    let interval = ev_field(&d.interval, "1", d.focus == EventField::Interval, pal, EventField::Interval);
    let unit = View::new(Style {
        size: Size { width: length(90.0_f32), height: length(34.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(d.repeat.unit().to_string(), 12.0, theme.fg_muted, Alignment::Start);
    let every = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![
        View::new(Style { size: Size { width: length(70.0_f32), height: Dimension::auto() }, ..Default::default() })
            .children(vec![interval]),
        unit,
    ]);
    out.push(labeled(theme, "Cada", every));

    // Días de la semana (sólo semanal).
    if d.repeat == Repeat::Weekly {
        let mut days: Vec<View<Msg>> = Vec::with_capacity(7);
        for i in 0..7u32 {
            days.push(day_toggle(theme, WD_INITIALS[i as usize], d.byday[i as usize], i));
        }
        let row = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(32.0_f32) },
            gap: Size { width: length(5.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .children(days);
        out.push(labeled(theme, "Días", row));
    }

    // Condición de término + campo contextual.
    let end_chip = cycle_chip(theme, d.repeat_end.label(), Msg::EventCycleRepeatEnd);
    let end_extra: Option<View<Msg>> = match d.repeat_end {
        RepeatEnd::Never => None,
        RepeatEnd::Count => {
            Some(ev_field(&d.count, "10", d.focus == EventField::Count, pal, EventField::Count))
        }
        RepeatEnd::Until => {
            Some(ev_field(&d.until, "AAAA-MM-DD", d.focus == EventField::Until, pal, EventField::Until))
        }
    };
    let mut end_children = vec![
        View::new(Style { size: Size { width: length(150.0_f32), height: Dimension::auto() }, ..Default::default() })
            .children(vec![end_chip]),
    ];
    if let Some(extra) = end_extra {
        end_children.push(View::new(Style { size: Size { width: Dimension::auto(), height: Dimension::auto() }, flex_grow: 1.0, ..Default::default() }).children(vec![extra]));
    }
    let end_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(end_children);
    out.push(labeled(theme, "Termina", end_row));

    out
}

/// Botón cuadrado de día de la semana, resaltado si está activo.
fn day_toggle(theme: &Theme, label: &str, on: bool, idx: u32) -> View<Msg> {
    let (bg, fg) = if on { (theme.accent, theme.bg_app) } else { (theme.bg_input, theme.fg_muted) };
    View::new(Style {
        size: Size { width: length(30.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::EventToggleByday(idx))
    .text(label, 12.0, fg)
}

/// Sección **Invitados**: pills removibles de los actuales + caja de alta +
/// sugerencias desde la libreta según lo tecleado.
fn attendees_section(model: &Model, pal: &TextInputPalette, d: &EventDraft) -> View<Msg> {
    let theme = &model.theme;
    let mut block: Vec<View<Msg>> = Vec::new();

    // Invitados actuales (uno por fila, con ✕ para quitar).
    for a in &d.attendees {
        block.push(attendee_pill(theme, a));
    }

    // Caja para sumar a mano (Enter agrega).
    block.push(ev_field(
        &d.invitee,
        "Nombre <correo>  · Enter",
        d.focus == EventField::Invitee,
        pal,
        EventField::Invitee,
    ));

    // Sugerencias: contactos con correo que matchean lo tecleado y no están ya.
    let query = d.invitee.text();
    if !query.trim().is_empty() {
        let invited: std::collections::HashSet<String> =
            d.attendees.iter().map(|a| a.email.to_lowercase()).collect();
        let mut shown = 0;
        for c in model.store_ref().search_contacts(&query) {
            if shown >= 4 {
                break;
            }
            let Some(email) = c.primary_email() else { continue };
            if invited.contains(&email.to_lowercase()) {
                continue;
            }
            block.push(suggestion_row(theme, &c.full_name, email));
            shown += 1;
        }
    }

    let inner = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        ..Default::default()
    })
    .children(block);
    labeled(theme, "Invitados", inner)
}

fn attendee_pill(theme: &Theme, a: &Address) -> View<Msg> {
    let label = match &a.name {
        Some(n) => format!("{n}  ·  {}", a.email),
        None => a.email.clone(),
    };
    let text = View::new(Style {
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(label, 12.5, theme.fg_text, Alignment::Start);
    let close = View::new(Style {
        size: Size { width: length(24.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text("✕", 12.0, theme.fg_muted);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: pad_xy(10.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::EventRemoveInvitee(a.email.clone()))
    .children(vec![text, close])
}

fn suggestion_row(theme: &Theme, name: &str, email: &str) -> View<Msg> {
    let texts = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![
        View::new(Style { size: Size { width: Dimension::auto(), height: percent(1.0_f32) }, align_items: Some(AlignItems::Center), ..Default::default() })
            .text(name.to_string(), 12.5, theme.fg_text),
        View::new(Style { size: Size { width: Dimension::auto(), height: percent(1.0_f32) }, flex_grow: 1.0, align_items: Some(AlignItems::Center), ..Default::default() })
            .text_aligned(email.to_string(), 11.0, theme.fg_muted, Alignment::Start),
    ]);
    let plus = View::new(Style {
        size: Size { width: length(20.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text("＋", 13.0, theme.accent);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        align_items: Some(AlignItems::Center),
        padding: pad_xy(10.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_input)
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::EventAddContact { name: name.to_string(), email: email.to_string() })
    .children(vec![texts, plus])
}

/// Chip que cicla un valor al hacer clic (muestra el valor + “⟳”).
fn cycle_chip(theme: &Theme, value: &str, msg: Msg) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        align_items: Some(AlignItems::Center),
        padding: pad_xy(10.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_input)
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(msg)
    .children(vec![
        View::new(Style { size: Size { width: Dimension::auto(), height: percent(1.0_f32) }, flex_grow: 1.0, align_items: Some(AlignItems::Center), ..Default::default() })
            .text_aligned(value.to_string(), 13.0, theme.fg_text, Alignment::Start),
        View::new(Style { size: Size { width: length(20.0_f32), height: percent(1.0_f32) }, align_items: Some(AlignItems::Center), ..Default::default() })
            .text_aligned("⟳".to_string(), 13.0, theme.fg_muted, Alignment::End),
    ])
}

/// Envoltorio común: backdrop oscuro + tarjeta centrada. Click en el backdrop
/// cierra; click en la tarjeta no se propaga (re-enfoca el campo activo es
/// trabajo del propio campo).
fn editor_card(theme: &Theme, title: &str, mut children: Vec<View<Msg>>) -> View<Msg> {
    let title_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        ..Default::default()
    })
    .text_aligned(title.to_string(), 16.0, theme.fg_text, Alignment::Start);
    let mut all = vec![title_view];
    all.append(&mut children);

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(EDITOR_W), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        padding: pad_xy(20.0, 18.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .on_click(Msg::Noop)
    .children(all);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 150))
    .on_click(Msg::CloseEditor)
    .children(vec![card])
}

/// Fila inferior de acciones: Eliminar (si es edición) · Cancelar · Guardar.
fn editor_actions(theme: &Theme, editing: bool, save: Msg, delete: Msg) -> View<Msg> {
    let mut row: Vec<View<Msg>> = Vec::new();
    if editing {
        row.push(button("Eliminar", theme.bg_button, theme.fg_destructive, delete));
    }
    row.push(spacer());
    row.push(button("Cancelar", theme.bg_button, theme.fg_text, Msg::CloseEditor));
    row.push(button("Guardar", theme.accent, theme.bg_app, save));
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        padding: Rect { left: length(0.0_f32), right: length(0.0_f32), top: length(6.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .children(row)
}

/// Campo con etiqueta arriba.
fn labeled(theme: &Theme, label: &str, field: View<Msg>) -> View<Msg> {
    let lbl = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .text_aligned(label.to_string(), 11.0, theme.fg_muted, Alignment::Start);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: Dimension::auto(), height: Dimension::auto() },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(3.0_f32) },
        ..Default::default()
    })
    .children(vec![lbl, field])
}

fn ev_field(state: &TextInputState, ph: &str, focused: bool, pal: &TextInputPalette, which: EventField) -> View<Msg> {
    text_input_view(state, ph, focused, pal, Msg::EventFocus(which))
}

fn ct_field(state: &TextInputState, ph: &str, focused: bool, pal: &TextInputPalette, which: ContactField) -> View<Msg> {
    text_input_view(state, ph, focused, pal, Msg::ContactFocus(which))
}

/// Casilla de verificación con etiqueta a la derecha.
fn checkbox(theme: &Theme, label: &str, on: bool, msg: Msg) -> View<Msg> {
    let (glyph, color) = if on { ("☑", theme.accent) } else { ("☐", theme.fg_muted) };
    let mark = View::new(Style {
        size: Size { width: length(20.0_f32), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(glyph, 16.0, color);
    let lbl = View::new(Style {
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(label.to_string(), 12.0, theme.fg_text, Alignment::Start);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: Dimension::auto(), height: length(26.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .on_click(msg)
    .children(vec![mark, lbl])
}

// ── primitivas ──────────────────────────────────────────────────────────────

fn status_bar(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(STATUS_H) },
        align_items: Some(AlignItems::Center),
        padding: pad_xy(14.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(model.status.clone(), 12.0, theme.fg_muted, Alignment::Start)
}

fn button(label: &str, bg: Color, fg: Color, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size { width: Dimension::auto(), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: pad_xy(12.0, 0.0),
        ..Default::default()
    })
    .fill(bg)
    .radius(6.0)
    .hover_fill(bg)
    .text(label, 13.0, fg)
    .on_click(msg)
}

fn spacer() -> View<Msg> {
    View::new(Style {
        size: Size { width: Dimension::auto(), height: length(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
}

fn hhmm(ts: i64) -> String {
    let (_, h, mi, _) = time::to_civil(ts);
    format!("{h:02}:{mi:02}")
}

fn avatar(initials: &str, seed: &str) -> View<Msg> {
    avatar_sized(initials, seed, AVATAR, 15.0)
}
fn avatar_big(initials: &str, seed: &str) -> View<Msg> {
    avatar_sized(initials, seed, 56.0, 22.0)
}
fn avatar_sized(initials: &str, seed: &str, size: f32, text: f32) -> View<Msg> {
    View::new(Style {
        size: Size { width: length(size), height: length(size) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(stable_color(seed))
    .radius((size / 2.0) as f64)
    .text(initials.to_string(), text, Color::from_rgba8(255, 255, 255, 235))
}

/// Mapa calendario → color: el hex declarado, o un color estable por id.
fn calendar_colors(model: &Model) -> HashMap<String, Color> {
    model
        .store_ref()
        .calendars()
        .iter()
        .map(|c| {
            let color = c.color.as_deref().and_then(parse_hex).unwrap_or_else(|| stable_color(&c.id));
            (c.id.clone(), color)
        })
        .collect()
}

fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::from_rgba8(r, g, b, 255))
}

/// Color estable derivado de una semilla (hash FNV-1a → paleta sobria).
fn stable_color(seed: &str) -> Color {
    const PALETTE: [(u8, u8, u8); 8] = [
        (94, 129, 172),
        (163, 109, 156),
        (122, 162, 110),
        (191, 138, 92),
        (108, 153, 168),
        (170, 120, 120),
        (130, 140, 175),
        (150, 150, 110),
    ];
    let mut h: u32 = 2_166_136_261;
    for b in seed.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16_777_619);
    }
    let (r, g, b) = PALETTE[(h as usize) % PALETTE.len()];
    Color::from_rgba8(r, g, b, 255)
}

impl Model {
    /// Acceso de sólo-lectura al store para la vista.
    pub(crate) fn store_ref(&self) -> &CalStore {
        &self.store
    }
}
