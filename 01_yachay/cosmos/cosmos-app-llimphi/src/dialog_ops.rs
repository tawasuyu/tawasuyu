//! Operaciones sobre los diálogos modales: crear contacto, crear carta
//! y configurar ubicación «Hoy».

use crate::dialog;
use crate::library;
use crate::model::Model;
use crate::nav_ops::{open_hoy_chart, refresh_nav};
use crate::update::save_ui;

// =====================================================================
// Abrir diálogos
// =====================================================================

/// Abre el diálogo de nuevo contacto bajo el grupo seleccionado (o su
/// grupo padre, o la raíz).
pub(crate) fn open_contact_dialog(m: &mut Model) {
    let group = m.selected_node().and_then(|n| match n.kind {
        library::NavKind::Group => library::parse_group_key(&n.key),
        _ => n.parent.as_deref().and_then(library::parse_group_key),
    });
    m.dialog = Some(dialog::Dialog::NewContact(dialog::NewContactForm {
        group,
        name: String::new(),
    }));
    m.dialog_field = dialog::DialogField::Name;
    m.dialog_input.set_text(String::new());
    m.menu_open = None;
    m.nav_ctx = None;
}

/// Abre el diálogo de nueva carta bajo el contacto seleccionado. Prefill
/// desde la carta de trabajo. Sin contacto destino → error.
pub(crate) fn open_chart_dialog(m: &mut Model) {
    use crate::model::Msg;
    let contact = m.selected_node().and_then(|n| match n.kind {
        library::NavKind::Contact => library::parse_contact_key(&n.key),
        library::NavKind::Chart => n.parent.as_deref().and_then(library::parse_contact_key),
        library::NavKind::Group => None,
    });
    let Some(contact) = contact else {
        m.error = Some("Nueva carta: seleccioná un contacto".into());
        return;
    };
    let bd = &m.chart.birth_data;
    m.dialog = Some(dialog::Dialog::NewChart(dialog::NewChartForm {
        contact,
        label: "Carta nueva".into(),
        date: format!("{:04}-{:02}-{:02}", bd.year, bd.month, bd.day),
        time: format!("{:02}:{:02}", bd.hour, bd.minute),
        city_query: String::new(),
        place: bd.birthplace_label.clone().unwrap_or_default(),
        lat: bd.latitude_deg,
        lon: bd.longitude_deg,
        tz: bd.tz_offset_minutes,
    }));
    m.dialog_field = dialog::DialogField::Label;
    m.dialog_input.set_text("Carta nueva".to_string());
    m.menu_open = None;
    m.nav_ctx = None;
}

// =====================================================================
// Interacción con campos del diálogo
// =====================================================================

/// Carga el valor del campo `f` en el buffer de edición y le da el foco.
pub(crate) fn dialog_focus(m: &mut Model, f: dialog::DialogField) {
    let v = m.dialog.as_ref().map(|d| d.field(f)).unwrap_or_default();
    m.dialog_field = f;
    m.dialog_input.set_text(v);
}

/// Aplica una ciudad del atlas al form de carta (autocompleta lat/lon/tz).
pub(crate) fn dialog_pick_city(m: &mut Model, idx: usize) {
    let Some(city) = dialog::CITY_PRESETS.get(idx) else { return };
    match m.dialog.as_mut() {
        Some(dialog::Dialog::NewChart(c)) => {
            c.place = city.name.to_string();
            c.lat = city.lat;
            c.lon = city.lon;
            c.tz = city.tz;
            c.city_query = city.name.to_string();
        }
        Some(dialog::Dialog::HoyLoc(c)) => {
            c.place = city.name.to_string();
            c.lat = format!("{:.4}", city.lat);
            c.lon = format!("{:.4}", city.lon);
            c.city_query = city.name.to_string();
            if c.label.trim().is_empty() {
                c.label = city.name.to_string();
            }
        }
        _ => {}
    }
    if m.dialog_field == dialog::DialogField::City {
        m.dialog_input.set_text(city.name.to_string());
    }
}

// =====================================================================
// Confirmar diálogo
// =====================================================================

/// Confirma el diálogo abierto: valida y crea en el store.
pub(crate) fn dialog_confirm(m: &mut Model) {
    match m.dialog.take() {
        Some(dialog::Dialog::NewContact(f)) => {
            let name = f.name.trim().to_string();
            if name.is_empty() {
                m.error = Some("El contacto necesita un nombre".into());
                m.dialog = Some(dialog::Dialog::NewContact(f));
                return;
            }
            match m.store.as_ref().map(|s| s.create_contact(f.group, &name, None)) {
                Some(Ok(c)) => {
                    if let Some(g) = f.group {
                        m.nav_expanded.insert(format!("g:{g}"));
                    }
                    refresh_nav(m);
                    m.nav_selected = Some(format!("c:{}", c.id));
                    m.status_note = Some(format!("Contacto creado: {name}"));
                }
                Some(Err(e)) => m.error = Some(format!("crear contacto: {e}")),
                None => {}
            }
        }
        Some(dialog::Dialog::NewChart(f)) => {
            let Some((y, mo, d)) = parse_date(&f.date) else {
                m.error = Some("Fecha inválida (usá AAAA-MM-DD)".into());
                m.dialog = Some(dialog::Dialog::NewChart(f));
                return;
            };
            let Some((h, mi)) = parse_time(&f.time) else {
                m.error = Some("Hora inválida (usá HH:MM)".into());
                m.dialog = Some(dialog::Dialog::NewChart(f));
                return;
            };
            let mut bd = m.chart.birth_data.clone();
            bd.year = y;
            bd.month = mo;
            bd.day = d;
            bd.hour = h;
            bd.minute = mi;
            bd.second = 0.0;
            bd.tz_offset_minutes = f.tz;
            bd.latitude_deg = f.lat;
            bd.longitude_deg = f.lon;
            bd.birthplace_label = if f.place.is_empty() {
                None
            } else {
                Some(f.place.clone())
            };
            let label = if f.label.trim().is_empty() {
                "Carta nueva"
            } else {
                f.label.trim()
            };
            let res = m.store.as_ref().map(|s| {
                s.create_chart(
                    f.contact,
                    cosmos_model::ChartKind::Natal,
                    label,
                    &bd,
                    &m.chart.config,
                    None,
                )
            });
            match res {
                Some(Ok(ch)) => {
                    m.nav_expanded.insert(format!("c:{}", f.contact));
                    refresh_nav(m);
                    m.status_note = Some(format!("Carta creada: {label}"));
                    crate::nav_ops::do_cargar(m, ch.id.to_string());
                }
                Some(Err(e)) => m.error = Some(format!("crear carta: {e}")),
                None => {}
            }
        }
        Some(dialog::Dialog::HoyLoc(f)) => {
            let lat: Option<f64> = f.lat.trim().parse().ok();
            let lon: Option<f64> = f.lon.trim().parse().ok();
            let (Some(lat), Some(lon)) = (lat, lon) else {
                m.error = Some("Elegí una ciudad o tecleá lat/lon válidas".into());
                m.dialog = Some(dialog::Dialog::HoyLoc(f));
                return;
            };
            let label = match (f.label.trim(), f.place.trim()) {
                (l, _) if !l.is_empty() => l.to_string(),
                (_, p) if !p.is_empty() => p.to_string(),
                _ => format!("{lat:.2}°, {lon:.2}°"),
            };
            let loc = crate::model::GeoLoc { label, lat, lon };
            match f.target {
                dialog::HoyTarget::User => {
                    m.cfg.user_location = Some(loc.clone());
                    save_ui(m);
                    refresh_nav(m);
                    open_hoy_chart(m, library::HOY_USER_KEY, &loc);
                    m.status_note = Some(format!("Ubicación fijada: {}", loc.label));
                }
                dialog::HoyTarget::Extra => {
                    m.cfg.hoy_locations.push(loc.clone());
                    let i = m.cfg.hoy_locations.len() - 1;
                    save_ui(m);
                    refresh_nav(m);
                    let key = library::hoy_loc_key(i);
                    m.nav_expanded.insert(library::HOY_CONTACT_KEY.to_string());
                    open_hoy_chart(m, &key, &loc);
                    m.status_note = Some(format!("Carta de hoy: {}", loc.label));
                }
            }
        }
        None => {}
    }
}

// =====================================================================
// Parsers auxiliares
// =====================================================================

/// Parsea `AAAA-MM-DD`.
pub(crate) fn parse_date(s: &str) -> Option<(i32, u32, u32)> {
    let p: Vec<&str> = s.trim().split('-').collect();
    if p.len() != 3 {
        return None;
    }
    let y = p[0].trim().parse().ok()?;
    let mo: u32 = p[1].trim().parse().ok()?;
    let d: u32 = p[2].trim().parse().ok()?;
    if (1..=12).contains(&mo) && (1..=31).contains(&d) {
        Some((y, mo, d))
    } else {
        None
    }
}

/// Parsea `HH:MM`.
pub(crate) fn parse_time(s: &str) -> Option<(u32, u32)> {
    let p: Vec<&str> = s.trim().split(':').collect();
    if p.len() != 2 {
        return None;
    }
    let h: u32 = p[0].trim().parse().ok()?;
    let mi: u32 = p[1].trim().parse().ok()?;
    if h < 24 && mi < 60 {
        Some((h, mi))
    } else {
        None
    }
}
