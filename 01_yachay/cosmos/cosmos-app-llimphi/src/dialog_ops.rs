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

/// Resuelve el contexto de creación desde el nodo seleccionado: contacto
/// destino + su nombre (para prefijar el combobox) + grupo donde aterriza un
/// contacto nuevo. Un grupo no fija contacto (se elige en el diálogo).
fn chart_context(m: &Model) -> (Option<cosmos_model::ContactId>, String, Option<cosmos_model::GroupId>) {
    let Some(n) = m.selected_node() else {
        return (None, String::new(), None);
    };
    match n.kind {
        library::NavKind::Contact => {
            let id = library::parse_contact_key(&n.key);
            let group = n.parent.as_deref().and_then(library::parse_group_key);
            (id, n.label.clone(), group)
        }
        library::NavKind::Chart => {
            // Subir al contacto padre por su clave para nombre + grupo.
            let ckey = n.parent.clone();
            let contact = ckey.as_deref().and_then(library::parse_contact_key);
            let (name, group) = ckey
                .as_deref()
                .and_then(|k| m.nav_nodes.iter().find(|nn| nn.key == k))
                .map(|cn| (cn.label.clone(), cn.parent.as_deref().and_then(library::parse_group_key)))
                .unwrap_or_default();
            (contact, name, group)
        }
        library::NavKind::Group => (None, String::new(), library::parse_group_key(&n.key)),
    }
}

/// Abre el diálogo de nueva carta. Si el nodo de origen es un contacto (o
/// una carta), lo preelige; si es un grupo, recuerda el grupo y deja el
/// contacto a elegir/crear en el combobox. Prefill de fecha/lugar desde la
/// carta de trabajo.
pub(crate) fn open_chart_dialog(m: &mut Model) {
    let (contact, contact_name, group) = chart_context(m);
    let bd = &m.chart.birth_data;
    m.dialog = Some(dialog::Dialog::NewChart(dialog::NewChartForm {
        contact,
        group,
        contact_query: contact_name,
        kind: cosmos_model::ChartKind::Natal,
        label: "Carta nueva".into(),
        date: format!("{:04}-{:02}-{:02}", bd.year, bd.month, bd.day),
        time: format!("{:02}:{:02}", bd.hour, bd.minute),
        city_query: String::new(),
        place: bd.birthplace_label.clone().unwrap_or_default(),
        lat: bd.latitude_deg,
        lon: bd.longitude_deg,
        tz: bd.tz_offset_minutes,
        tz_iana: String::new(),
        kind_open: false,
        cal_open: false,
        cal_year: bd.year,
        cal_month: bd.month.clamp(1, 12),
    }));
    // Si no hay contacto preelegido, arrancar en el combobox de contacto;
    // si lo hay, en la etiqueta.
    m.dialog_field = if contact.is_none() {
        dialog::DialogField::Contact
    } else {
        dialog::DialogField::Label
    };
    m.dialog_input.set_text(if contact.is_none() {
        String::new()
    } else {
        "Carta nueva".to_string()
    });
    m.menu_open = None;
    m.nav_ctx = None;
}

// =====================================================================
// Interacción con el diálogo de carta (combobox / tipo / calendario / hora)
// =====================================================================

/// Elige un contacto existente en el combobox.
pub(crate) fn dialog_pick_contact(m: &mut Model, id: cosmos_model::ContactId) {
    let name = m
        .nav_nodes
        .iter()
        .find(|n| library::parse_contact_key(&n.key) == Some(id))
        .map(|n| n.label.clone())
        .unwrap_or_default();
    if let Some(dialog::Dialog::NewChart(c)) = m.dialog.as_mut() {
        c.contact = Some(id);
        c.contact_query = name.clone();
    }
    if m.dialog_field == dialog::DialogField::Contact {
        m.dialog_input.set_text(name);
    }
}

/// Fija el tipo de carta y cierra la lista.
pub(crate) fn dialog_set_kind(m: &mut Model, kind: cosmos_model::ChartKind) {
    if let Some(dialog::Dialog::NewChart(c)) = m.dialog.as_mut() {
        c.kind = kind;
        c.kind_open = false;
    }
}

/// Despliega/cierra la lista de tipos.
pub(crate) fn dialog_toggle_kind(m: &mut Model) {
    if let Some(dialog::Dialog::NewChart(c)) = m.dialog.as_mut() {
        c.kind_open = !c.kind_open;
    }
}

/// Despliega/cierra el calendario inline (sincroniza su mes con la fecha).
pub(crate) fn dialog_toggle_calendar(m: &mut Model) {
    if let Some(dialog::Dialog::NewChart(c)) = m.dialog.as_mut() {
        c.cal_open = !c.cal_open;
        if c.cal_open {
            if let Some((y, mo, _)) = parse_date(&c.date) {
                c.cal_year = y;
                c.cal_month = mo;
            }
        }
    }
}

/// Día elegido en el calendario: escribe la fecha y cierra el calendario.
pub(crate) fn dialog_cal_pick(m: &mut Model, y: i32, mo: u32, d: u32) {
    if let Some(dialog::Dialog::NewChart(c)) = m.dialog.as_mut() {
        c.date = format!("{y:04}-{mo:02}-{d:02}");
        c.cal_year = y;
        c.cal_month = mo;
        c.cal_open = false;
    }
    if m.dialog_field == dialog::DialogField::Date {
        let txt = m.dialog.as_ref().map(|dg| dg.field(dialog::DialogField::Date)).unwrap_or_default();
        m.dialog_input.set_text(txt);
    }
}

/// Cambia el mes/año en foco del calendario.
pub(crate) fn dialog_cal_view(m: &mut Model, y: i32, mo: u32) {
    if let Some(dialog::Dialog::NewChart(c)) = m.dialog.as_mut() {
        c.cal_year = y;
        c.cal_month = mo;
    }
}

/// Ajusta hora/minuto con wrap (HH 0..23, MM 0..59).
pub(crate) fn dialog_time_step(m: &mut Model, hours: bool, delta: i32) {
    if let Some(dialog::Dialog::NewChart(c)) = m.dialog.as_mut() {
        let (h, mi) = parse_time(&c.time).unwrap_or((0, 0));
        let (mut h, mut mi) = (h as i32, mi as i32);
        if hours {
            h = (h + delta).rem_euclid(24);
        } else {
            mi = (mi + delta).rem_euclid(60);
        }
        c.time = format!("{h:02}:{mi:02}");
    }
    if m.dialog_field == dialog::DialogField::Time {
        let txt = m.dialog.as_ref().map(|dg| dg.field(dialog::DialogField::Time)).unwrap_or_default();
        m.dialog_input.set_text(txt);
    }
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

/// Aplica una ciudad del atlas al form (autocompleta lat/lon + zona). `idx`
/// indexa el mismo resultado de búsqueda que pintó las filas — el handler
/// re-corre la búsqueda sobre la query del form para resolverlo.
pub(crate) fn dialog_pick_city(m: &mut Model, idx: usize) {
    // Re-resolver la ciudad desde la query del form activo.
    let query = match m.dialog.as_ref() {
        Some(dialog::Dialog::NewChart(c)) => c.city_query.clone(),
        Some(dialog::Dialog::HoyLoc(c)) => c.city_query.clone(),
        _ => return,
    };
    let Some(city) = dialog::city_matches(&query).into_iter().nth(idx) else { return };
    let label = city.label();
    match m.dialog.as_mut() {
        Some(dialog::Dialog::NewChart(c)) => {
            c.place = label.clone();
            c.lat = city.lat;
            c.lon = city.lon;
            c.tz_iana = city.tz.to_string();
            // Offset histórico para la fecha/hora actuales del form (se
            // recomputa al confirmar contra la fecha final).
            if let Some(off) = offset_for(&c.tz_iana, &c.date, &c.time) {
                c.tz = off;
            }
            c.city_query = label.clone();
        }
        Some(dialog::Dialog::HoyLoc(c)) => {
            c.place = label.clone();
            c.lat = format!("{:.4}", city.lat);
            c.lon = format!("{:.4}", city.lon);
            c.city_query = label.clone();
            if c.label.trim().is_empty() {
                c.label = label.clone();
            }
        }
        _ => {}
    }
    if m.dialog_field == dialog::DialogField::City {
        m.dialog_input.set_text(label);
    }
}

/// Offset UTC en minutos de una zona IANA para `date`+`time` (`AAAA-MM-DD`,
/// `HH:MM`). `None` si algo no parsea.
fn offset_for(tz_iana: &str, date: &str, time: &str) -> Option<i32> {
    if tz_iana.is_empty() {
        return None;
    }
    let (y, mo, d) = parse_date(date)?;
    let (h, mi) = parse_time(time)?;
    let naive = chrono::NaiveDate::from_ymd_opt(y, mo, d)?.and_hms_opt(h, mi, 0)?;
    cosmos_cities::offset_minutes_at(tz_iana, naive)
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
            // Resolver el contacto destino: el elegido, o crear uno nuevo
            // con el nombre tecleado (bajo el grupo recordado).
            let contact = match f.contact {
                Some(id) => id,
                None => {
                    let name = f.contact_query.trim();
                    if name.is_empty() {
                        m.error = Some("Nueva carta: elegí o nombrá un contacto".into());
                        m.dialog = Some(dialog::Dialog::NewChart(f));
                        return;
                    }
                    match m.store.as_ref().map(|s| s.create_contact(f.group, name, None)) {
                        Some(Ok(c)) => {
                            if let Some(g) = f.group {
                                m.nav_expanded.insert(format!("g:{g}"));
                            }
                            c.id
                        }
                        Some(Err(e)) => {
                            m.error = Some(format!("crear contacto: {e}"));
                            m.dialog = Some(dialog::Dialog::NewChart(f));
                            return;
                        }
                        None => return,
                    }
                }
            };
            let mut bd = m.chart.birth_data.clone();
            bd.year = y;
            bd.month = mo;
            bd.day = d;
            bd.hour = h;
            bd.minute = mi;
            bd.second = 0.0;
            // Offset autoritativo: recomputado de la zona IANA contra la
            // fecha final (con el DST de la época). Si no hubo ciudad con
            // zona, cae al offset que tenía el form.
            bd.tz_offset_minutes = offset_for(&f.tz_iana, &f.date, &f.time).unwrap_or(f.tz);
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
                s.create_chart(contact, f.kind, label, &bd, &m.chart.config, None)
            });
            match res {
                Some(Ok(ch)) => {
                    m.nav_expanded.insert(format!("c:{contact}"));
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
