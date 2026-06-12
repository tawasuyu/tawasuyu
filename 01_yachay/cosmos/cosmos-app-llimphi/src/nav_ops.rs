//! Operaciones sobre el árbol de navegación: selección, apertura de cartas,
//! creación/renombrado/corte/pegado/borrado de nodos.

use crate::engine;
use crate::library;
use crate::model::{GeoLoc, Model, Msg, OpenTab};
use crate::persist;

use super::dialog;

// =====================================================================
// Refrescar árbol
// =====================================================================

/// Reconstruye el snapshot del árbol: la rama fija «Efemérides → Hoy»
/// (sintética, desde la config) al tope, luego el store.
pub(crate) fn refresh_nav(m: &mut Model) {
    let mut nodes = library::hoy_nodes(&m.cfg.user_location, &m.cfg.hoy_locations);
    if let Some(s) = &m.store {
        nodes.extend(library::snapshot(s));
    }
    m.nav_nodes = nodes;
}

// =====================================================================
// Selección y apertura de cartas
// =====================================================================

/// Render puntual de una carta con las opciones globales actuales.
pub(crate) fn compute_render(m: &Model, chart: &cosmos_model::Chart) -> cosmos_render::RenderModel {
    crate::engine::compute(chart, &m.overlays, m.harmonic, m.cfg.minor_aspects, m.rectify_offset_min).0
}

/// Activa la carta-pestaña `i`: la vuelve la carta de trabajo y recomputa.
pub(crate) fn activate_tab(m: &mut Model, i: usize) {
    let Some(tab) = m.open.get(i) else { return };
    m.active_tab = i;
    m.chart = tab.chart.clone();
    m.selected_card = tab.id.clone();
    if let Some(id) = &tab.id {
        m.nav_selected = Some(format!("h:{id}"));
    }
    crate::persist::save_chart_to_disk(&m.chart);
    crate::update::recompute_chart(m);
    crate::update::recompute_astro(m);
}

pub(crate) fn close_chart_tab(m: &mut Model, i: usize) {
    if i >= m.open.len() {
        return;
    }
    m.open.remove(i);
    if m.open.is_empty() {
        // Nunca quedamos sin carta: re-abrimos la de trabajo como scratch.
        m.open.push(OpenTab {
            id: None,
            chart: m.chart.clone(),
            render: m.render.clone(),
        });
        activate_tab(m, 0);
        return;
    }
    let new = if m.active_tab > i {
        m.active_tab - 1
    } else if m.active_tab >= m.open.len() {
        m.open.len() - 1
    } else {
        m.active_tab
    };
    activate_tab(m, new);
}

/// El nodo seleccionado interpretado como ids del store (según su tipo).
pub(crate) fn nav_click(m: &mut Model, key: String) {
    m.nav_selected = Some(key.clone());
    match m.node(&key).map(|n| n.kind) {
        Some(library::NavKind::Chart) => {
            if library::is_hoy_chart_key(&key) {
                hoy_select(m, &key);
            } else if let Some(id) = m.node(&key).and_then(|n| n.chart_id.clone()) {
                m.hoy_active = None;
                do_cargar(m, id);
            }
        }
        Some(_) => m.toggle_nav(key),
        None => {}
    }
}

/// Click en una carta de la rama «Hoy»: abre la carta del instante actual
/// en esa ubicación.
fn hoy_select(m: &mut Model, key: &str) {
    if key == library::HOY_USER_KEY {
        match m.cfg.user_location.clone() {
            Some(loc) => open_hoy_chart(m, key, &loc),
            None => open_where_am_i(m),
        }
    } else if let Some(i) = library::parse_hoy_loc_key(key) {
        if let Some(loc) = m.cfg.hoy_locations.get(i).cloned() {
            open_hoy_chart(m, key, &loc);
        }
    }
}

/// Abre (o refresca) la carta «ahora» de una ubicación de «Hoy» como
/// pestaña, reusando la pestaña por su clave sintética.
pub(crate) fn open_hoy_chart(m: &mut Model, key: &str, loc: &GeoLoc) {
    let chart = engine::now_chart(&loc.label, loc.lat, loc.lon);
    let render = compute_render(m, &chart);
    if let Some(i) = m.open.iter().position(|t| t.id.as_deref() == Some(key)) {
        m.open[i].chart = chart.clone();
        m.open[i].render = render.clone();
        m.active_tab = i;
    } else {
        m.open.push(OpenTab {
            id: Some(key.to_string()),
            chart: chart.clone(),
            render: render.clone(),
        });
        m.active_tab = m.open.len() - 1;
    }
    m.chart = chart;
    m.render = render;
    m.selected_card = Some(key.to_string());
    m.nav_selected = Some(key.to_string());
    m.hoy_active = Some(key.to_string());
    crate::update::recompute_astro(m);
}

/// Abre el diálogo «¿Dónde estoy?» para configurar la ubicación del usuario.
pub(crate) fn open_where_am_i(m: &mut Model) {
    m.dialog = Some(dialog::Dialog::HoyLoc(dialog::HoyLocForm {
        target: dialog::HoyTarget::User,
        label: "Mi ubicación".into(),
        city_query: String::new(),
        place: String::new(),
        lat: String::new(),
        lon: String::new(),
    }));
    m.dialog_field = dialog::DialogField::City;
    m.dialog_input.set_text(String::new());
    m.menu_open = None;
    m.nav_ctx = None;
}

/// Abre el diálogo «carta de hoy por coordenadas» (se agrega bajo «Hoy»).
pub(crate) fn open_add_hoy(m: &mut Model) {
    m.dialog = Some(dialog::Dialog::HoyLoc(dialog::HoyLocForm {
        target: dialog::HoyTarget::Extra,
        label: String::new(),
        city_query: String::new(),
        place: String::new(),
        lat: String::new(),
        lon: String::new(),
    }));
    m.dialog_field = dialog::DialogField::City;
    m.dialog_input.set_text(String::new());
    m.menu_open = None;
    m.nav_ctx = None;
}

/// Carga una carta del store por su id (string ULID) como pestaña.
pub(crate) fn do_cargar(m: &mut Model, id: String) {
    if let Some(i) = m.open.iter().position(|t| t.id.as_deref() == Some(id.as_str())) {
        activate_tab(m, i);
        return;
    }
    let chart = m
        .store
        .as_ref()
        .and_then(|s| id.parse().ok().and_then(|cid| s.get_chart(cid).ok()));
    if let Some(chart) = chart {
        let render = compute_render(m, &chart);
        m.open.push(OpenTab {
            id: Some(id),
            chart,
            render,
        });
        let i = m.open.len() - 1;
        activate_tab(m, i);
    } else {
        m.error = Some(format!("no se pudo cargar carta: {id}"));
    }
}

/// Abre una carta de ejemplo como pestaña nueva (scratch, sin id).
pub(crate) fn do_nueva(m: &mut Model) {
    let chart = crate::engine::sample_chart();
    let render = compute_render(m, &chart);
    m.open.push(OpenTab {
        id: None,
        chart,
        render,
    });
    let i = m.open.len() - 1;
    activate_tab(m, i);
    m.status_note = Some("Carta de ejemplo abierta".into());
}

/// Duplica la carta de trabajo como una carta nueva del store.
pub(crate) fn do_duplicar(m: &mut Model) {
    let contact = m.selected_node().and_then(|n| match n.kind {
        library::NavKind::Contact => library::parse_contact_key(&n.key),
        library::NavKind::Chart => n.parent.as_deref().and_then(library::parse_contact_key),
        library::NavKind::Group => None,
    });
    let Some(cid) = contact else {
        m.error = Some("Duplicar: seleccioná una carta o un contacto".into());
        return;
    };
    let label = format!("{} (copia)", m.chart.label);
    let res = m.store.as_ref().map(|s| {
        s.create_chart(
            cid,
            cosmos_model::ChartKind::Natal,
            &label,
            &m.chart.birth_data,
            &m.chart.config,
            None,
        )
    });
    match res {
        Some(Ok(ch)) => {
            m.nav_expanded.insert(format!("c:{cid}"));
            m.nav_selected = Some(format!("h:{}", ch.id));
            refresh_nav(m);
            m.status_note = Some(format!("Carta duplicada: {label}"));
        }
        Some(Err(e)) => m.error = Some(format!("duplicar: {e}")),
        None => {}
    }
}

/// Persiste la carta de trabajo en el store.
pub(crate) fn do_guardar(m: &mut Model) {
    let sel = m.selected_node().map(|n| (n.kind, n.key.clone()));
    match sel {
        Some((library::NavKind::Chart, key)) => {
            let Some(id) = library::parse_chart_key(&key) else { return };
            let res = m.store.as_ref().map(|s| {
                s.update_chart(id, &m.chart.label, &m.chart.birth_data, &m.chart.config)
            });
            match res {
                Some(Ok(())) => {
                    refresh_nav(m);
                    m.status_note = Some(format!("Carta guardada: {}", m.chart.label));
                }
                Some(Err(e)) => m.error = Some(format!("guardar: {e}")),
                None => {}
            }
        }
        Some((library::NavKind::Contact, key)) => {
            let Some(cid) = library::parse_contact_key(&key) else { return };
            let res = m.store.as_ref().map(|s| {
                s.create_chart(
                    cid,
                    cosmos_model::ChartKind::Natal,
                    &m.chart.label,
                    &m.chart.birth_data,
                    &m.chart.config,
                    None,
                )
            });
            match res {
                Some(Ok(ch)) => {
                    m.nav_expanded.insert(key.clone());
                    m.selected_card = Some(ch.id.to_string());
                    m.nav_selected = Some(format!("h:{}", ch.id));
                    // La pestaña activa (scratch) queda ligada a la carta nueva.
                    if let Some(t) = m.open.get_mut(m.active_tab) {
                        t.id = Some(ch.id.to_string());
                        t.chart = m.chart.clone();
                    }
                    refresh_nav(m);
                    m.status_note = Some(format!("Carta creada: {}", m.chart.label));
                }
                Some(Err(e)) => m.error = Some(format!("guardar: {e}")),
                None => {}
            }
        }
        _ => {
            m.error =
                Some("Guardar: seleccioná una carta (sobrescribe) o un contacto (crea)".into())
        }
    }
}

// =====================================================================
// Crear / modificar nodos del árbol
// =====================================================================

pub(crate) fn new_group(m: &mut Model) {
    let Some(store) = &m.store else { return };
    let parent = m
        .selected_node()
        .and_then(|n| library::parse_group_key(&n.key));
    match store.create_group(parent, "Grupo nuevo", None) {
        Ok(g) => {
            if let Some(pk) = m.nav_selected.clone() {
                m.nav_expanded.insert(pk);
            }
            refresh_nav(m);
            start_rename(m, format!("g:{}", g.id));
        }
        Err(e) => m.error = Some(format!("crear grupo: {e}")),
    }
}

/// El grupo del store que corresponde a la selección actual.
fn selected_group(m: &Model) -> Option<(cosmos_model::GroupId, String)> {
    let mut key = m.nav_selected.clone()?;
    loop {
        let node = m.node(&key)?;
        if node.kind == library::NavKind::Group {
            return library::parse_group_key(&node.key).map(|id| (id, node.label.clone()));
        }
        key = node.parent.clone()?;
    }
}

/// Exporta el grupo seleccionado a un archivo JSON elegido con el diálogo
/// nativo Guardar.
pub(crate) fn do_export_group(m: &mut Model) {
    let Some(store) = &m.store else { return };
    let Some((gid, name)) = selected_group(m) else {
        m.error = Some("Seleccioná un grupo para exportar".into());
        return;
    };
    let export = persist::build_group_export(store, gid, &name);
    let default_name = format!("{}.json", name.replace(['/', '\\'], "_"));
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("JSON", &["json"])
        .set_file_name(&default_name)
        .save_file()
    {
        match persist::write_group_file(&path, &export) {
            Ok(()) => m.status_note = Some(format!("Grupo exportado a {}", path.display())),
            Err(e) => m.error = Some(format!("exportar: {e}")),
        }
    }
}

/// Importa un grupo de contactos desde un archivo JSON elegido con el
/// diálogo nativo Abrir.
pub(crate) fn do_import_group(m: &mut Model) {
    let Some(store) = m.store.clone() else { return };
    let parent = selected_group(m).map(|(g, _)| g);
    let Some(path) = rfd::FileDialog::new()
        .add_filter("JSON", &["json"])
        .pick_file()
    else {
        return;
    };
    match persist::read_group_file(&path) {
        Ok(g) => match persist::import_group_into(&store, parent, &g) {
            Ok(()) => {
                refresh_nav(m);
                m.nav_expanded = library::container_keys(&m.nav_nodes).into_iter().collect();
                m.status_note = Some(format!("Grupo «{}» importado", g.name));
            }
            Err(e) => m.error = Some(format!("importar: {e}")),
        },
        Err(e) => m.error = Some(format!("importar: {e}")),
    }
}

pub(crate) fn delete_selected(m: &mut Model) {
    let Some(store) = &m.store else { return };
    let Some(node) = m.selected_node() else { return };
    let key = node.key.clone();
    match node.kind {
        library::NavKind::Group => {
            if let Some(id) = library::parse_group_key(&key) {
                library::delete_group_recursive(store, id);
            }
        }
        library::NavKind::Contact => {
            if let Some(id) = library::parse_contact_key(&key) {
                library::delete_contact_recursive(store, id);
            }
        }
        library::NavKind::Chart => {
            if let Some(id) = library::parse_chart_key(&key) {
                let _ = store.delete_chart(id);
            }
        }
    }
    m.nav_selected = None;
    refresh_nav(m);
    m.status_note = Some("Elemento eliminado".into());
}

/// Mueve el nodo cortado (`nav_cut`) bajo el grupo seleccionado (o a la
/// raíz si no hay selección). Grupos y contactos se mueven con
/// `store.move_*`; las cartas no (no hay move_chart).
pub(crate) fn paste_node(m: &mut Model) {
    let Some(cut_key) = m.nav_cut.clone() else {
        m.error = Some("Pegar: nada cortado".into());
        return;
    };
    if library::parse_chart_key(&cut_key).is_some() {
        m.error = Some("Pegar: las cartas no se mueven entre contactos".into());
        return;
    }
    let target_group = match m.selected_node().map(|n| (n.kind, n.key.clone())) {
        None => None,
        Some((library::NavKind::Group, key)) => library::parse_group_key(&key),
        Some(_) => {
            m.error = Some("Pegar: elegí un grupo destino (o deseleccioná para la raíz)".into());
            return;
        }
    };
    if let Some(gid) = library::parse_group_key(&cut_key) {
        if Some(gid) == target_group {
            m.error = Some("Pegar: destino inválido".into());
            return;
        }
    }
    let res = m.store.as_ref().map(|s| {
        if let Some(gid) = library::parse_group_key(&cut_key) {
            s.move_group(gid, target_group)
        } else if let Some(cid) = library::parse_contact_key(&cut_key) {
            s.move_contact(cid, target_group)
        } else {
            Ok(())
        }
    });
    match res {
        Some(Ok(())) => {
            if let Some(g) = target_group {
                m.nav_expanded.insert(format!("g:{g}"));
            }
            m.nav_cut = None;
            refresh_nav(m);
            m.status_note = Some("Movido".into());
        }
        Some(Err(e)) => m.error = Some(format!("mover: {e}")),
        None => {}
    }
}

pub(crate) fn start_rename(m: &mut Model, key: String) {
    let current = m.node(&key).map(|n| n.label.clone()).unwrap_or_default();
    m.rename_input.set_text(current);
    m.nav_selected = Some(key.clone());
    m.nav_rename = Some(key);
}

pub(crate) fn commit_rename(m: &mut Model) {
    let Some(key) = m.nav_rename.take() else { return };
    let name = m.rename_input.text();
    if let Some(store) = &m.store {
        if name.trim().is_empty() {
            return;
        }
        let r = if let Some(id) = library::parse_group_key(&key) {
            store.rename_group(id, &name)
        } else if let Some(id) = library::parse_contact_key(&key) {
            store.rename_contact(id, &name)
        } else if let Some(id) = library::parse_chart_key(&key) {
            store.rename_chart(id, &name)
        } else {
            Ok(())
        };
        if let Err(e) = r {
            m.error = Some(format!("renombrar: {e}"));
        }
    }
    refresh_nav(m);
}
