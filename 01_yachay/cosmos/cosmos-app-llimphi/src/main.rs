//! `cosmos-app-llimphi` — shell astronómico/astrológico sobre Llimphi.
//!
//! IDE de cartas: barra de menú principal arriba (`Archivo`/`Vista`/
//! `Capas`/`Armónico`/`Ayuda`), árbol de navegación a la izquierda
//! (biblioteca de cartas + catálogo de gráficas astrológicas y
//! astronómicas), pestañas en el área central (una por gráfica abierta)
//! y barra de estado abajo. Click derecho sobre la rueda abre un menú
//! contextual con las opciones del wheel. Todo lo configurable vive en la
//! vista `Configuración` y en los menús `Capas`/`Armónico`.
//!
//! Módulos: `model` (estado + mensajes + taxonomías), `persist`
//! (UI-state + cartas + watcher), `engine` (compose del wheel),
//! `astroview` (cómputo + gráficas astronómicas), `view` (paneles
//! astrológicos), `chrome` (menú/árbol/pestañas/estado/contextuales),
//! `astrocarto` (mapa equirectangular), `format` (símbolos). Acá queda el
//! `impl App` y la lógica de transición.

mod astrocarto;
mod astroview;
mod chrome;
mod dialog;
mod engine;
mod format;
mod glyphs;
mod library;
mod model;
mod persist;
mod print;
mod tools;
mod view;

use std::sync::Arc;

use cosmos_engine::Corpus;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, FlexDirection, Size, Style};
use llimphi_ui::{App, DragPhase, Handle, Key, KeyState, NamedKey, View};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use wawa_config_llimphi::theme_from_wawa;

use crate::astroview::compute_astro;
use crate::chrome::MenuCmd;
use crate::engine::{compute, sample_chart};
use crate::model::{MenuKind, Model, Msg, OpenTab, WheelOpt};
use crate::persist::{
    load_chart_from_disk, load_ui_state, save_chart_to_disk, save_ui_state, spawn_chart_watcher,
    UiState,
};

const CORPUS_DEFAULT_RON: &str = include_str!("../../cosmos-corpus/ejemplo.ron");

struct Cosmos;

// =====================================================================
// Helpers de transición (reusados por mensajes directos y menú)
// =====================================================================

/// Recomputa el render de TODAS las cartas abiertas (mosaico siempre
/// consistente al cambiar capas/armónico) y refresca `m.render` con el de
/// la pestaña activa. Las cartas abiertas son pocas; el costo es marginal.
fn recompute_chart(m: &mut Model) {
    let off = m.rectify_offset_min;
    if m.open.is_empty() {
        let (render, error) = compute(&m.chart, &m.overlays, m.harmonic, m.cfg.minor_aspects, off);
        m.render = render;
        m.error = error;
        return;
    }
    let overlays = m.overlays.clone();
    let (h, minor) = (m.harmonic, m.cfg.minor_aspects);
    let active = m.active_tab.min(m.open.len() - 1);
    for i in 0..m.open.len() {
        let (render, error) = compute(&m.open[i].chart, &overlays, h, minor, off);
        m.open[i].render = render;
        if i == active {
            m.render = m.open[i].render.clone();
            m.error = error;
        }
    }
}

/// Render puntual de una carta con las opciones globales actuales.
fn compute_render(m: &Model, chart: &cosmos_model::Chart) -> cosmos_render::RenderModel {
    compute(chart, &m.overlays, m.harmonic, m.cfg.minor_aspects, m.rectify_offset_min).0
}

// El cómputo astronómico es el pesado (144 muestras × 10 cuerpos): NO corre
// en el hilo de UI. Esto sólo marca sucio; el despacho a un worker ocurre al
// final de `update` (que tiene el Handle). El render de la carta sí es barato
// y queda síncrono (ver `recompute_chart`).
fn recompute_astro(m: &mut Model) {
    m.astro_dirty = true;
}

/// Activa la carta-pestaña `i`: la vuelve la carta de trabajo y recomputa.
fn activate_tab(m: &mut Model, i: usize) {
    let Some(tab) = m.open.get(i) else { return };
    m.active_tab = i;
    m.chart = tab.chart.clone();
    m.selected_card = tab.id.clone();
    if let Some(id) = &tab.id {
        m.nav_selected = Some(format!("h:{id}"));
    }
    save_chart_to_disk(&m.chart);
    recompute_chart(m);
    recompute_astro(m);
}

fn close_chart_tab(m: &mut Model, i: usize) {
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

fn set_harmonic(m: &mut Model, h: u32) {
    if m.harmonic != h {
        m.harmonic = h;
        recompute_chart(m);
    }
}

fn apply_overlay(m: &mut Model, k: model::OverlayKind) {
    if let Some(idx) = m.overlays.iter().position(|x| *x == k) {
        m.overlays.remove(idx);
    } else {
        m.overlays.push(k);
    }
    recompute_chart(m);
}

fn toggle_wheel(m: &mut Model, opt: WheelOpt) {
    match opt {
        WheelOpt::MinorAspects => {
            m.cfg.minor_aspects = !m.cfg.minor_aspects;
            // Los menores deben calcularse para poder dibujarse.
            recompute_chart(m);
        }
        WheelOpt::CoordLabels => m.cfg.coord_labels = !m.cfg.coord_labels,
        WheelOpt::Dial3d => m.cfg.dial_3d = !m.cfg.dial_3d,
        WheelOpt::AscCross => m.cfg.asc_cross = !m.cfg.asc_cross,
    }
}

/// Carga una carta del store por su id (string ULID) como pestaña: si ya
/// está abierta, salta a ella; si no, la abre en una pestaña nueva.
fn do_cargar(m: &mut Model, id: String) {
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
fn do_nueva(m: &mut Model) {
    let chart = sample_chart();
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

/// Duplica la carta de trabajo como una carta nueva del store, bajo el
/// contacto del nodo seleccionado (o el contacto padre de la carta sel.).
fn do_duplicar(m: &mut Model) {
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

/// Persiste la carta de trabajo en el store. Si hay una carta
/// seleccionada en el árbol, la sobrescribe; si hay un contacto
/// seleccionado, crea una carta nueva bajo él.
fn do_guardar(m: &mut Model) {
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

/// Reconstruye el snapshot del árbol desde el store (tras una mutación).
fn refresh_nav(m: &mut Model) {
    if let Some(s) = &m.store {
        m.nav_nodes = library::snapshot(s);
    }
}

/// El nodo seleccionado interpretado como ids del store (según su tipo).
fn nav_click(m: &mut Model, key: String) {
    m.nav_selected = Some(key.clone());
    match m.node(&key).map(|n| n.kind) {
        Some(library::NavKind::Chart) => {
            if let Some(id) = m.node(&key).and_then(|n| n.chart_id.clone()) {
                do_cargar(m, id);
            }
        }
        Some(_) => m.toggle_nav(key),
        None => {}
    }
}

fn new_group(m: &mut Model) {
    let Some(store) = &m.store else { return };
    // Bajo el grupo seleccionado si lo hay; si no, a la raíz.
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


fn delete_selected(m: &mut Model) {
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
fn paste_node(m: &mut Model) {
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

fn start_rename(m: &mut Model, key: String) {
    let current = m.node(&key).map(|n| n.label.clone()).unwrap_or_default();
    m.rename_input.set_text(current);
    m.nav_selected = Some(key.clone());
    m.nav_rename = Some(key);
}

fn commit_rename(m: &mut Model) {
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

/// Aplica una selección del segmented de tema (0 = Oscuro, 1 = Claro,
/// 2 = Impresión) y refleja el `Theme` activo en el modelo. La selección
/// manual gana sobre el tinte de wawa-config (mismo criterio que antes:
/// elegir tema a mano fija el preset puro).
fn set_theme_mode(m: &mut Model, idx: usize) {
    m.cfg.set_theme_idx(idx);
    m.theme = m.cfg.active_theme();
}

/// Rasteriza la hoja imprimible (rueda + cabecera + aspectos) a un PNG de
/// alta resolución con el mismo motor que pinta la pantalla — fidelidad
/// gráfica — y la abre en el visor de imágenes del SO para imprimir.
fn do_imprimir(m: &mut Model) {
    match crate::print::imprimir_carta(m) {
        Ok(path) => {
            m.status_note = Some(format!("Hoja rasterizada y abierta para imprimir ({})", path.display()));
        }
        Err(e) => m.error = Some(format!("imprimir: {e}")),
    }
}

fn do_recargar(m: &mut Model) {
    if let Some(c) = load_chart_from_disk() {
        m.chart = c;
        recompute_chart(m);
        recompute_astro(m);
        m.status_note = Some("Carta recargada de disco".into());
    }
}

/// Elimina el nodo seleccionado del árbol (carta/contacto/grupo) — misma
/// ruta que el botón 🗑 del explorador.
fn do_eliminar(m: &mut Model) {
    delete_selected(m);
}

fn apply_cmd(m: &mut Model, cmd: MenuCmd) {
    match cmd {
        MenuCmd::Sep => {}
        MenuCmd::Nueva => do_nueva(m),
        MenuCmd::Guardar => do_guardar(m),
        MenuCmd::Theme(idx) => set_theme_mode(m, idx),
        MenuCmd::Imprimir => do_imprimir(m),
        MenuCmd::Duplicar => do_duplicar(m),
        MenuCmd::Recargar => do_recargar(m),
        MenuCmd::Eliminar => do_eliminar(m),
        MenuCmd::SetChartView(cv) => m.chart_view = cv,
        MenuCmd::GoToolCat(tc) => {
            // Activa la categoría en el sidebar donde vive (o la trae al
            // derecho si no está acoplada en ningún lado).
            let item = model::DockItem::from_tool_cat(tc);
            if m.dock_left.contains(&item) {
                m.active_left = Some(item);
            } else {
                m.dock_move(item, model::DockSide::Right);
            }
            m.tools_open = true;
        }
        MenuCmd::ToggleNav => m.nav_open = !m.nav_open,
        MenuCmd::ToggleTools => m.tools_open = !m.tools_open,
        MenuCmd::Overlay(k) => apply_overlay(m, k),
        MenuCmd::Harmonic(h) => set_harmonic(m, h),
        MenuCmd::AcercaDe => {
            m.status_note =
                Some("cosmos · astronomía + astrología sobre Llimphi (wgpu + vello + taffy)".into())
        }
        MenuCmd::Wheel(opt) => toggle_wheel(m, opt),
        MenuCmd::Deselect => m.selected_body = None,
    }
}

/// Ejecuta una acción del menú contextual del árbol sobre el nodo ya
/// seleccionado (lo dejó `OpenNavCtx`).
fn apply_nav_act(m: &mut Model, act: chrome::NavAct) {
    use chrome::NavAct;
    match act {
        NavAct::NewGroup => new_group(m),
        NavAct::NewContact => open_contact_dialog(m),
        NavAct::NewChart => open_chart_dialog(m),
        NavAct::Rename => {
            if let Some(key) = m.nav_selected.clone() {
                start_rename(m, key);
            }
        }
        NavAct::Cut => {
            m.nav_cut = m.nav_selected.clone();
            if m.nav_cut.is_some() {
                m.status_note = Some("Cortado — elegí un grupo destino y pegá".into());
            }
        }
        NavAct::Paste => paste_node(m),
        NavAct::Duplicate => do_duplicar(m),
        NavAct::Delete => delete_selected(m),
    }
}

// =====================================================================
// Rectificador de hora (direcciones primarias)
// =====================================================================

/// Corre el barrido de rectificación con los eventos cargados (±2 h).
fn run_rectify(m: &mut Model) {
    if m.rectify_events.is_empty() {
        m.error = Some("Rectificador: cargá al menos un evento (edad)".into());
        return;
    }
    let eventos: Vec<cosmos_engine::EventoConocido> = m
        .rectify_events
        .iter()
        .map(|&edad_years| cosmos_engine::EventoConocido { edad_years })
        .collect();
    let key = rectify_key(m);
    match cosmos_engine::rectificar(&m.chart, &eventos, 120, key) {
        Ok(res) => {
            let secs = res.mejor_offset_segundos;
            m.status_note = Some(format!(
                "Rectificación: {:+} s ({:+} min) · error {:.2}",
                secs,
                secs / 60,
                res.mejor_puntaje
            ));
            m.rectify_result = Some(res);
        }
        Err(e) => m.error = Some(format!("rectificar: {e}")),
    }
}

/// Clave arco↔año para el motor.
fn rectify_key(m: &Model) -> &'static str {
    if m.rectify_naibod {
        "naibod"
    } else {
        "ptolemy"
    }
}

/// Calcula los triggers GR (contactos directo/converso) a la edad de
/// inspección, con la carta y el offset de jog actuales.
fn compute_triggers(m: &mut Model) {
    let req = cosmos_engine::PipelineRequest::PrimaryDirections {
        target_age_years: m.rectify_age,
        key: rectify_key(m).to_string(),
    };
    match cosmos_engine::compose(&m.chart, m.rectify_offset_min, &[req]) {
        Ok(r) => {
            m.rectify_triggers = r.gr_triggers;
            if m.rectify_triggers.is_empty() {
                m.status_note = Some(format!("Sin triggers GR a los {:.1} años", m.rectify_age));
            }
        }
        Err(e) => m.error = Some(format!("triggers GR: {e}")),
    }
}

/// Aplica el mejor offset hallado a la hora de nacimiento de la carta.
fn apply_rectify(m: &mut Model) {
    let Some(res) = &m.rectify_result else {
        m.error = Some("Rectificador: corré primero el barrido".into());
        return;
    };
    let secs = res.mejor_offset_segundos;
    let bd = &mut m.chart.birth_data;
    // Total de segundos del día + offset, normalizado a [0, 86400).
    let total = ((bd.hour as i64 * 60 + bd.minute as i64) * 60) + bd.second as i64 + secs;
    let total = total.rem_euclid(86_400);
    bd.hour = (total / 3600) as u32;
    bd.minute = ((total % 3600) / 60) as u32;
    bd.second = (total % 60) as f64;
    bd.time_certainty = cosmos_model::TimeCertainty::Exact;
    // Refleja en la pestaña activa, persiste y recomputa con offset 0.
    if let Some(t) = m.open.get_mut(m.active_tab) {
        t.chart = m.chart.clone();
    }
    m.rectify_offset_min = 0;
    m.rectify_result = None;
    save_chart_to_disk(&m.chart);
    recompute_chart(m);
    recompute_astro(m);
    m.status_note = Some(format!(
        "Hora rectificada: {:02}:{:02}:{:02}",
        m.chart.birth_data.hour, m.chart.birth_data.minute, m.chart.birth_data.second as u32
    ));
}

// =====================================================================
// Diálogos modales (crear contacto / crear carta)
// =====================================================================

/// Abre el diálogo de nuevo contacto bajo el grupo seleccionado (o su
/// grupo padre, o la raíz).
fn open_contact_dialog(m: &mut Model) {
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

/// Abre el diálogo de nueva carta bajo el contacto seleccionado (o el
/// contacto padre de la carta seleccionada). Prefill desde la carta de
/// trabajo. Sin contacto destino → error.
fn open_chart_dialog(m: &mut Model) {
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

/// Carga el valor del campo `f` en el buffer de edición y le da el foco.
fn dialog_focus(m: &mut Model, f: dialog::DialogField) {
    let v = m.dialog.as_ref().map(|d| d.field(f)).unwrap_or_default();
    m.dialog_field = f;
    m.dialog_input.set_text(v);
}

/// Aplica una ciudad del atlas al form de carta (autocompleta lat/lon/tz).
fn dialog_pick_city(m: &mut Model, idx: usize) {
    let Some(city) = dialog::CITY_PRESETS.get(idx) else { return };
    if let Some(dialog::Dialog::NewChart(c)) = m.dialog.as_mut() {
        c.place = city.name.to_string();
        c.lat = city.lat;
        c.lon = city.lon;
        c.tz = city.tz;
        c.city_query = city.name.to_string();
    }
    if m.dialog_field == dialog::DialogField::City {
        m.dialog_input.set_text(city.name.to_string());
    }
}

/// Confirma el diálogo abierto: valida y crea en el store.
fn dialog_confirm(m: &mut Model) {
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
                    do_cargar(m, ch.id.to_string());
                }
                Some(Err(e)) => m.error = Some(format!("crear carta: {e}")),
                None => {}
            }
        }
        None => {}
    }
}

/// Parsea `AAAA-MM-DD`.
fn parse_date(s: &str) -> Option<(i32, u32, u32)> {
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
fn parse_time(s: &str) -> Option<(u32, u32)> {
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

fn save_ui(m: &Model) {
    save_ui_state(&UiState {
        overlays: m.overlays.clone(),
        harmonic: m.harmonic,
        cfg: m.cfg.clone(),
        nav_w: m.nav_w,
        tools_w: m.tools_w,
        nav_open: m.nav_open,
        tools_open: m.tools_open,
        chart_view: m.chart_view,
        tool_cat: m.tool_cat,
        expanded_panels: m.expanded_panels.clone(),
        tile_mode: m.tile_mode,
        dock_left: m.dock_left.clone(),
        dock_right: m.dock_right.clone(),
        sphere_yaw: m.sphere_yaw,
        sphere_pitch: m.sphere_pitch,
        sky_nadir: m.sky_nadir,
    });
}

impl App for Cosmos {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "cosmos · canvas (llimphi)"
    }

    /// El `app_id` Wayland: pata lo usa para correlacionar foco ↔ dientes en el
    /// rail hospedado, así que el `HostClient` registra con este mismo string.
    fn app_id() -> Option<&'static str> {
        Some("gioser.cosmos")
    }

    fn initial_size() -> (u32, u32) {
        (1200, 860)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let cfg_wawa = wawa_config::WawaConfig::load();
        let _ = rimay_localize::set_locale(&cfg_wawa.lang);

        let handle_clone = handle.clone();
        let watcher = wawa_config::ConfigWatcher::spawn(move |new_cfg| {
            handle_clone.dispatch(Msg::WawaConfigChanged(Box::new(new_cfg)));
        })
        .map_err(|e| eprintln!("cosmos · wawa-config watcher: {e}"))
        .ok();

        let chart = load_chart_from_disk().unwrap_or_else(|| {
            let c = sample_chart();
            save_chart_to_disk(&c);
            c
        });
        let ui = load_ui_state();
        // En modo impresión el tema B/N gana y no acepta el tinte de
        // wawa-config (la hoja tiene que ser blanca sí o sí). En claro/
        // oscuro, el tinte del SO se aplica como siempre.
        let theme = if ui.cfg.print_mode {
            ui.cfg.active_theme()
        } else {
            let base = if ui.cfg.theme_dark {
                Theme::dark()
            } else {
                Theme::light()
            };
            theme_from_wawa(&cfg_wawa, &base)
        };
        // El render de la carta es barato → síncrono. El astro (orto/ocaso/
        // efemérides) es el caro: arranca en `None` ("calculando…") y se
        // computa en un worker que reentra con `AstroComputed`. `init` corre
        // en winit DESPUÉS de crear la ventana, así que un cómputo pesado aquí
        // congelaría la ventana recién abierta. Generación 1 = la del arranque.
        let (render, error) = compute(&chart, &ui.overlays, ui.harmonic, ui.cfg.minor_aspects, 0);
        let astro = None;
        {
            let (c, use_now) = (chart.clone(), ui.cfg.use_now);
            handle.spawn(move || Msg::AstroComputed(1, Arc::new(compute_astro(&c, use_now))));
        }
        let corpus = Corpus::desde_ron(CORPUS_DEFAULT_RON).unwrap_or_default();
        let chart_watcher = spawn_chart_watcher(handle);

        // Árbol de datos sobre cosmos-store: abrir, sembrar/migrar y armar
        // el snapshot jerárquico. Todo expandido en la primera carga.
        let store = library::open_store();
        if let Some(s) = &store {
            library::ensure_seed(s, &chart);
        }
        let nav_nodes = store.as_ref().map(library::snapshot).unwrap_or_default();
        let nav_expanded = library::container_keys(&nav_nodes).into_iter().collect();

        // Una pestaña inicial con la carta de trabajo (scratch, sin id).
        let open = vec![OpenTab {
            id: None,
            chart: chart.clone(),
            render: render.clone(),
        }];

        // Rail hospedado: si `COSMOS_DELEGATE_SIDEBAR` está set, cosmos delega su
        // sidebar a pata — publica sus dientes y queda puro canvas. El callback
        // (en el hilo lector del cliente) reinyecta las activaciones al bucle Elm.
        let delegated = std::env::var_os("COSMOS_DELEGATE_SIDEBAR").is_some();
        let host = if delegated {
            let teeth: Vec<pata_host::HostedTooth> = ui
                .dock_left
                .iter()
                .chain(&ui.dock_right)
                .map(|i| dock_item_tooth(*i))
                .collect();
            let h = handle.clone();
            pata_host::HostClient::connect("gioser.cosmos", "Cosmos", teeth, move |id| {
                h.dispatch(Msg::HostActivate(id))
            })
        } else {
            None
        };

        Model {
            chart,
            overlays: ui.overlays,
            harmonic: ui.harmonic,
            render,
            astro,
            astro_dirty: false,
            astro_gen: 1,
            corpus,
            cfg: ui.cfg,
            theme,
            error,
            status_note: None,
            open,
            active_tab: 0,
            tile_mode: ui.tile_mode,
            selected_card: None,
            selected_body: None,
            store,
            nav_nodes,
            nav_expanded,
            nav_selected: None,
            nav_rename: None,
            rename_input: llimphi_widget_text_input::TextInputState::new(),
            nav_cut: None,
            sphere_yaw: ui.sphere_yaw,
            sphere_pitch: ui.sphere_pitch,
            sky_nadir: ui.sky_nadir,
            wheel_zoom: 1.0,
            wheel_pan: (0.0, 0.0),
            carto_rect: Arc::new(std::sync::Mutex::new(None)),
            viewport: model::VIEWPORT,
            tools_scroll: 0.0,
            nav_w: ui.nav_w,
            tools_w: ui.tools_w,
            nav_open: ui.nav_open,
            tools_open: ui.tools_open,
            chart_view: ui.chart_view,
            tool_cat: ui.tool_cat,
            expanded_panels: ui.expanded_panels,
            active_left: ui.dock_left.first().copied(),
            active_right: ui.dock_right.first().copied(),
            dock_expanded: None,
            dock_left: ui.dock_left,
            dock_right: ui.dock_right,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: llimphi_motion::Tween::idle(1.0),
            ctx_open: None,
            nav_ctx: None,
            nav_scroll: 0.0,
            rectify_offset_min: 0,
            rectify_events: Vec::new(),
            rectify_result: None,
            rectify_naibod: true,
            rectify_age: 30.0,
            rectify_triggers: Vec::new(),
            dialog: None,
            dialog_field: dialog::DialogField::Name,
            dialog_input: llimphi_widget_text_input::TextInputState::new(),
            delegated,
            _host: host,
            _wawa_watcher: watcher,
            _chart_watcher: chart_watcher,
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        let mut persist = false;
        // Cualquier interacción que no sea abrir un menú limpia la nota
        // efímera de estado. El resultado del worker (AstroComputed) tampoco
        // la toca: es un evento de fondo, no una acción del usuario.
        match &msg {
            Msg::OpenMenu(_) | Msg::MenuTick | Msg::WawaConfigChanged(_) | Msg::AstroComputed(..) => {}
            _ => m.status_note = None,
        }
        match msg {
            Msg::WawaConfigChanged(cfg) => {
                // El modo impresión ignora el tinte del SO: la hoja es B/N.
                if !m.cfg.print_mode {
                    m.theme = theme_from_wawa(&cfg, &m.theme);
                }
                if cfg.lang != rimay_localize::current_locale() {
                    let _ = rimay_localize::set_locale(&cfg.lang);
                }
            }
            // multi-carta (tabs del centro)
            Msg::ActivateChartTab(i) => activate_tab(&mut m, i),
            Msg::CloseChartTab(i) => close_chart_tab(&mut m, i),
            Msg::ToggleTileMode => {
                m.tile_mode = !m.tile_mode;
                persist = true;
            }
            Msg::SphereRotate(dyaw, dpitch) => {
                // Sin persistir: el drag dispara muchos por segundo; evita
                // escribir el UI-state a disco en cada movimiento.
                m.sphere_yaw = (m.sphere_yaw + dyaw).rem_euclid(360.0);
                m.sphere_pitch = (m.sphere_pitch + dpitch).clamp(-89.0, 89.0);
            }
            Msg::SphereReset => {
                m.sphere_yaw = 26.0;
                m.sphere_pitch = -64.0;
                persist = true;
            }
            Msg::WheelPan(dx, dy) => {
                m.wheel_pan.0 += dx;
                m.wheel_pan.1 += dy;
            }
            Msg::WheelZoom(factor) => {
                m.wheel_zoom = (m.wheel_zoom * factor).clamp(0.25, 8.0);
            }
            Msg::WheelResetView => {
                m.wheel_zoom = 1.0;
                m.wheel_pan = (0.0, 0.0);
            }
            Msg::WheelSetView(z, px, py) => {
                m.wheel_zoom = z;
                m.wheel_pan = (px, py);
            }
            Msg::ToggleSkyNadir => {
                m.sky_nadir = !m.sky_nadir;
                persist = true;
            }
            Msg::Resized(w, h) => m.viewport = (w, h),
            Msg::ToolsScroll(delta) => {
                // El panel de herramientas que scrollea es la categoría
                // activa (derecha primero, si no izquierda).
                let cat = m
                    .dock_active(model::DockSide::Right)
                    .and_then(|i| i.tool_cat())
                    .or_else(|| m.dock_active(model::DockSide::Left).and_then(|i| i.tool_cat()));
                let content = cat.map(|c| tools::tools_content_h(c, &m)).unwrap_or(0.0);
                let viewport = tools::tools_viewport_h(&m);
                m.tools_scroll = llimphi_widget_scroll::clamp_offset(
                    m.tools_scroll + delta,
                    content,
                    viewport,
                );
            }
            // navegación
            Msg::ToggleNavNode(key) => m.toggle_nav(key),
            Msg::NavClick(key) => nav_click(&mut m, key),
            Msg::NewGroup => new_group(&mut m),
            Msg::DeleteSelected => delete_selected(&mut m),
            Msg::CutNode => {
                m.nav_cut = m.nav_selected.clone();
                if m.nav_cut.is_some() {
                    m.status_note = Some("Cortado — seleccioná un grupo destino y pegá".into());
                }
            }
            Msg::PasteNode => {
                paste_node(&mut m);
                persist = true;
            }
            Msg::RenameStart => {
                if let Some(key) = m.nav_selected.clone() {
                    start_rename(&mut m, key);
                }
            }
            Msg::RenameKey(ev) => {
                if m.nav_rename.is_some() {
                    m.rename_input.apply_key(&ev);
                }
            }
            Msg::RenameCommit => commit_rename(&mut m),
            Msg::RenameCancel => m.nav_rename = None,
            Msg::ChartFileChanged => {
                if let Some(c) = load_chart_from_disk() {
                    m.chart = c.clone();
                    // Reflejar la edición externa en la pestaña activa.
                    if let Some(t) = m.open.get_mut(m.active_tab) {
                        t.chart = c;
                    }
                    recompute_chart(&mut m);
                    recompute_astro(&mut m);
                }
            }
            Msg::SelectBody(sel) => {
                m.selected_body = if m.selected_body == sel { None } else { sel };
            }
            // capas / armónico / configuración
            Msg::ToggleOverlay(k) => {
                apply_overlay(&mut m, k);
                persist = true;
            }
            Msg::SetHarmonic(n) => {
                set_harmonic(&mut m, n);
                persist = true;
            }
            Msg::SetThemeMode(idx) => {
                set_theme_mode(&mut m, idx);
                persist = true;
            }
            Msg::PrintSheet => do_imprimir(&mut m),
            Msg::ToggleWheelOpt(opt) => {
                toggle_wheel(&mut m, opt);
                persist = true;
            }
            Msg::SetRotOffset(dv) => {
                m.cfg.rot_offset_deg = (m.cfg.rot_offset_deg + dv).rem_euclid(360.0);
                persist = true;
            }
            Msg::SetUseNow(b) => {
                m.cfg.use_now = b;
                recompute_astro(&mut m);
                persist = true;
            }
            // menú principal
            Msg::OpenMenu(k) => {
                m.menu_open = if m.menu_open == Some(k) { None } else { Some(k) };
                m.menu_active = usize::MAX;
                m.ctx_open = None;
                // Animación de aparición/swap: cada vez que se abre (o se
                // cambia de) menú, el dropdown se funde+desliza de nuevo.
                if m.menu_open.is_some() {
                    m.menu_anim = llimphi_motion::Tween::new(
                        0.0,
                        1.0,
                        llimphi_motion::motion::FAST,
                        llimphi_motion::motion::ease_out_cubic,
                    );
                    llimphi_motion::animate(handle, llimphi_motion::motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuPick(kind, idx) => {
                m.menu_open = None;
                m.menu_active = usize::MAX;
                let cmd = chrome::menu_entries(kind, &m).get(idx).map(|e| e.cmd);
                if let Some(cmd) = cmd {
                    apply_cmd(&mut m, cmd);
                    persist = true;
                }
            }
            Msg::MenuNav(dir) => {
                if let Some(kind) = m.menu_open {
                    let entries = chrome::menu_entries(kind, &m);
                    let items: Vec<_> = entries.iter().map(chrome::MenuEntry::to_item).collect();
                    m.menu_active =
                        llimphi_widget_context_menu::step_active(&items, m.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(kind) = m.menu_open {
                    let idx = m.menu_active;
                    let cmd = chrome::menu_entries(kind, &m).get(idx).map(|e| e.cmd);
                    m.menu_open = None;
                    m.menu_active = usize::MAX;
                    if let Some(cmd) = cmd {
                        apply_cmd(&mut m, cmd);
                        persist = true;
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::CloseMenu => {
                m.menu_open = None;
                m.menu_active = usize::MAX;
            }
            // menú contextual
            Msg::OpenCanvasCtx(x, y) => {
                m.ctx_open = Some((x, y));
                m.menu_open = None;
            }
            Msg::CtxPick(idx) => {
                m.ctx_open = None;
                let cmd = chrome::ctx_entries(&m).get(idx).map(|e| e.cmd);
                if let Some(cmd) = cmd {
                    apply_cmd(&mut m, cmd);
                    persist = true;
                }
            }
            Msg::CloseCtx => {
                m.ctx_open = None;
                m.nav_ctx = None;
            }
            // menú contextual del árbol de datos
            Msg::OpenNavCtx(key) => {
                m.nav_selected = Some(key.clone());
                m.nav_ctx = Some(key);
                m.ctx_open = None;
                m.menu_open = None;
            }
            Msg::NavCtxPick(idx) => {
                let act = m
                    .nav_ctx
                    .as_ref()
                    .map(|k| chrome::nav_ctx_entries(&m, k))
                    .and_then(|entries| entries.get(idx).and_then(|e| e.act));
                m.nav_ctx = None;
                if let Some(act) = act {
                    apply_nav_act(&mut m, act);
                    persist = true;
                }
            }
            Msg::NavScroll(delta) => {
                let content = chrome::nav_content_h(&m);
                let viewport = chrome::nav_viewport_h(&m);
                m.nav_scroll =
                    llimphi_widget_scroll::clamp_offset(m.nav_scroll + delta, content, viewport);
            }
            // rectificador de hora
            Msg::RectifyNudge(d) => {
                m.rectify_offset_min += d;
                recompute_chart(&mut m);
                recompute_astro(&mut m);
            }
            Msg::RectifyResetOffset => {
                m.rectify_offset_min = 0;
                recompute_chart(&mut m);
                recompute_astro(&mut m);
            }
            Msg::RectifyAddEvent => m.rectify_events.push(25.0),
            Msg::RectifyEventDelta(i, d) => {
                if let Some(e) = m.rectify_events.get_mut(i) {
                    *e = (*e + d).clamp(0.0, 120.0);
                }
            }
            Msg::RectifyRemoveEvent(i) => {
                if i < m.rectify_events.len() {
                    m.rectify_events.remove(i);
                }
            }
            Msg::RectifyRun => run_rectify(&mut m),
            Msg::RectifyApply => apply_rectify(&mut m),
            Msg::RectifySetKey(naibod) => {
                m.rectify_naibod = naibod;
                if !m.rectify_triggers.is_empty() {
                    compute_triggers(&mut m);
                }
            }
            Msg::RectifyAgeDelta(d) => {
                m.rectify_age = (m.rectify_age + d).clamp(0.0, 120.0);
            }
            Msg::RectifyTriggers => compute_triggers(&mut m),
            // diálogos modales
            Msg::OpenNewContactDialog => open_contact_dialog(&mut m),
            Msg::OpenNewChartDialog => open_chart_dialog(&mut m),
            Msg::DialogFocus(f) => dialog_focus(&mut m, f),
            Msg::DialogKey(ev) => {
                m.dialog_input.apply_key(&ev);
                let txt = m.dialog_input.text();
                let f = m.dialog_field;
                if let Some(d) = m.dialog.as_mut() {
                    d.set_field(f, txt);
                }
            }
            Msg::DialogPickCity(idx) => dialog_pick_city(&mut m, idx),
            Msg::DialogConfirm => {
                dialog_confirm(&mut m);
                persist = true;
            }
            Msg::DialogCancel => m.dialog = None,
            // layout guardable
            Msg::SetNavWidth(dx) => m.nudge_nav(dx),
            Msg::SetToolsWidth(dx) => m.nudge_tools(dx),
            Msg::PersistLayout => persist = true,
            // panel de herramientas
            Msg::ToggleToolPanel(p) => {
                m.toggle_panel(p);
                persist = true;
            }
            // dock
            Msg::DockActivate(side, item) => {
                // Clic en el diente activo del lado ya desplegado → colapsa
                // (toggle, estilo web); cualquier otro → activa + despliega.
                let toggle_off = m.dock_active(side) == Some(item)
                    && m.dock_expanded == Some(side);
                match side {
                    model::DockSide::Left => m.active_left = Some(item),
                    model::DockSide::Right => m.active_right = Some(item),
                }
                m.dock_expanded = if toggle_off { None } else { Some(side) };
                persist = true;
            }
            Msg::DockDrop(side, payload) => {
                if let Some(item) = model::DockItem::from_u64(payload) {
                    // Sólo mover si cambia de lado — evita el reordenado
                    // molesto al soltar (o al hacer clic) en el mismo lado.
                    let already = match side {
                        model::DockSide::Left => m.dock_left.contains(&item),
                        model::DockSide::Right => m.dock_right.contains(&item),
                    };
                    if !already {
                        m.dock_move(item, side);
                        persist = true;
                    }
                }
            }
            // Rail hospedado: pata reenvió el clic de un diente prestado. Mapea el
            // id al DockItem, deduce el lado por dónde vive, y togglea ese panel
            // (mismo comportamiento que DockActivate) — así aparece/desaparece
            // sobre el canvas de cosmos.
            Msg::HostActivate(id) => {
                if let Some(item) = model::DockItem::from_u64(id as u64) {
                    let side = if m.dock_left.contains(&item) {
                        model::DockSide::Left
                    } else {
                        model::DockSide::Right
                    };
                    let toggle_off =
                        m.dock_active(side) == Some(item) && m.dock_expanded == Some(side);
                    match side {
                        model::DockSide::Left => m.active_left = Some(item),
                        model::DockSide::Right => m.active_right = Some(item),
                    }
                    m.dock_expanded = if toggle_off { None } else { Some(side) };
                    persist = true;
                }
            }
            // tipo de gráfica
            Msg::SetChartView(v) => {
                m.chart_view = v;
                persist = true;
            }
            // Resultado del worker astronómico. Se aplica sólo si su
            // generación sigue vigente (si no, un recálculo posterior ya lo
            // dejó viejo y lo descartamos). `try_unwrap` recupera el dueño sin
            // copiar: el `Arc` llega con refcount 1 porque el Msg no se clona.
            Msg::AstroComputed(gen, astro) => {
                if gen == m.astro_gen {
                    m.astro = Some(Arc::try_unwrap(astro).unwrap_or_else(|a| (*a).clone()));
                }
            }
        }
        if persist {
            save_ui(&m);
        }
        // Cómputo astronómico FUERA del hilo de UI. Si algo lo marcó sucio,
        // bumpeamos la generación y lo despachamos a un worker; el resultado
        // reentra como `AstroComputed` y la UI sigue respondiendo (muestra el
        // astro previo —o "calculando…"— hasta que llega).
        if m.astro_dirty {
            m.astro_dirty = false;
            m.astro_gen = m.astro_gen.wrapping_add(1);
            let gen = m.astro_gen;
            let (c, use_now) = (m.chart.clone(), m.cfg.use_now);
            handle.spawn(move || Msg::AstroComputed(gen, Arc::new(compute_astro(&c, use_now))));
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let menu = chrome::menu_bar(model, &theme);
        let status = chrome::status_bar(model, &theme);
        let sp = SplitterPalette::from_theme(&theme);

        let center = chrome::center_view(model, &theme);

        // Dock: los **rails** flotan como overlay sobre el centro (los
        // dibuja `center_view`), así la rueda usa todo el hueco. Acá sólo
        // colocamos los **paneles** de contenido en panes resizables; la
        // barra azul queda pegada al panel. Angosto → sólo rails; clic en
        // un diente despliega ese lado (estilo web).
        let collapsed = chrome::dock_collapsed(model);
        // En modo delegado los rails los pinta pata; el panel de un lado aparece
        // sólo cuando ese lado está expandido (un diente hospedado activo) →
        // sin nada activo, cosmos es puro canvas.
        let (left_show, right_show) = if model.delegated {
            (
                model.dock_expanded == Some(model::DockSide::Left),
                model.dock_expanded == Some(model::DockSide::Right),
            )
        } else {
            (
                !collapsed || model.dock_expanded == Some(model::DockSide::Left),
                !collapsed || model.dock_expanded == Some(model::DockSide::Right),
            )
        };
        let left_panel = if left_show {
            chrome::dock_panel_for(model::DockSide::Left, model, &theme)
        } else {
            None
        };
        let right_panel = if right_show {
            chrome::dock_panel_for(model::DockSide::Right, model, &theme)
        } else {
            None
        };

        let mut core = center;
        if let Some(rp) = right_panel {
            core = splitter_two(
                Direction::Row,
                core,
                PaneSize::Flex,
                rp,
                PaneSize::Fixed(model.tools_w),
                |phase, dx| match phase {
                    DragPhase::Move => Some(Msg::SetToolsWidth(dx)),
                    DragPhase::End => Some(Msg::PersistLayout),
                },
                &sp,
            );
        }
        if let Some(lp) = left_panel {
            core = splitter_two(
                Direction::Row,
                lp,
                PaneSize::Fixed(model.nav_w),
                core,
                PaneSize::Flex,
                |phase, dx| match phase {
                    DragPhase::Move => Some(Msg::SetNavWidth(dx)),
                    DragPhase::End => Some(Msg::PersistLayout),
                },
                &sp,
            );
        }
        let body = core;

        let body_box = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            min_size: Size {
                width: llimphi_ui::llimphi_layout::taffy::prelude::length(0.0_f32),
                height: llimphi_ui::llimphi_layout::taffy::prelude::length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![body]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![menu, body_box, status])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        // El diálogo modal tiene prioridad sobre los menús.
        dialog::dialog_overlay(model, &model.theme).or_else(|| chrome::overlay_view(model, &model.theme))
    }

    fn on_key(model: &Model, ev: &llimphi_ui::KeyEvent) -> Option<Msg> {
        // Un diálogo modal captura el teclado: Enter confirma, Escape
        // cancela, el resto alimenta el campo enfocado.
        if model.dialog.is_some() {
            if ev.state == KeyState::Pressed {
                match &ev.key {
                    Key::Named(NamedKey::Enter) => return Some(Msg::DialogConfirm),
                    Key::Named(NamedKey::Escape) => return Some(Msg::DialogCancel),
                    _ => {}
                }
            }
            return Some(Msg::DialogKey(ev.clone()));
        }
        // Renombrar un nodo del árbol captura el teclado: Enter confirma,
        // Escape cancela, el resto alimenta el buffer de texto.
        if model.nav_rename.is_some() {
            if ev.state == KeyState::Pressed {
                match &ev.key {
                    Key::Named(NamedKey::Enter) => return Some(Msg::RenameCommit),
                    Key::Named(NamedKey::Escape) => return Some(Msg::RenameCancel),
                    _ => {}
                }
            }
            return Some(Msg::RenameKey(ev.clone()));
        }
        if ev.state != KeyState::Pressed {
            return None;
        }
        // Menú principal abierto: las flechas navegan. ←/→ cambian de menú
        // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc
        // cierra. El context-menu de la rueda queda mouse-only (sólo Esc).
        if let Some(kind) = model.menu_open {
            let order = MenuKind::order();
            let n = order.len().max(1);
            let cur = order.iter().position(|k| *k == kind).unwrap_or(0);
            return match &ev.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenu),
                Key::Named(NamedKey::ArrowLeft) => {
                    Some(Msg::OpenMenu(order[(cur + n - 1) % n]))
                }
                Key::Named(NamedKey::ArrowRight) => Some(Msg::OpenMenu(order[(cur + 1) % n])),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                _ => None,
            };
        }
        match &ev.key {
            Key::Named(NamedKey::Escape) => {
                if model.menu_open.is_some() {
                    Some(Msg::CloseMenu)
                } else if model.ctx_open.is_some() {
                    Some(Msg::CloseCtx)
                } else {
                    None
                }
            }
            Key::Character(s) if ev.modifiers.ctrl && s.as_str().eq_ignore_ascii_case("w") => {
                Some(Msg::CloseChartTab(model.active_tab))
            }
            // Ctrl+S → guardar carta en biblioteca (espeja Archivo/Editar).
            // Resolvemos el índice contra la misma lista que pinta el menú
            // para no acoplar el atajo al orden de las entradas.
            Key::Character(s) if ev.modifiers.ctrl && s.as_str().eq_ignore_ascii_case("s") => {
                chrome::menu_entries(MenuKind::Archivo, model)
                    .iter()
                    .position(|e| matches!(e.cmd, MenuCmd::Guardar))
                    .map(|i| Msg::MenuPick(MenuKind::Archivo, i))
            }
            _ => None,
        }
    }

    fn on_resize(_model: &Model, width: u32, height: u32) -> Option<Msg> {
        Some(Msg::Resized(width as f32, height as f32))
    }

    /// Rueda del ratón sobre el lienzo central: zoom (rueda sola), paneo
    /// vertical (Ctrl) y paneo horizontal (Alt). El árbol y el panel de
    /// herramientas **consumen** la rueda por su cuenta (scroll propio),
    /// así que cuando este handler global se invoca el cursor está sobre
    /// el área gráfica — no hace falta gating por coordenadas (que fallaba
    /// al maximizar / en HiDPI).
    fn on_wheel(
        model: &Model,
        delta: llimphi_ui::WheelDelta,
        cursor: (f32, f32),
        modifiers: llimphi_ui::Modifiers,
    ) -> Option<Msg> {
        const STEP: f32 = 40.0;
        if modifiers.ctrl {
            Some(Msg::WheelPan(0.0, -delta.y * STEP))
        } else if modifiers.alt {
            Some(Msg::WheelPan(-delta.y * STEP, 0.0))
        } else {
            // Zoom: rueda hacia arriba (delta.y < 0) acerca.
            let factor = if delta.y < 0.0 { 1.12 } else { 0.892 };
            // En astrocarto, el zoom va HACIA el cursor: ajusta el paneo
            // para que el punto del mapa bajo el puntero quede fijo. Sólo
            // ahí conocemos el rect del lienzo (lo dejó el `paint_with`).
            // En el Cielo el lienzo es una cúpula radial centrada: basta
            // ajustar el paneo para que el punto bajo el cursor quede fijo
            // (no hace falta la escala base, sólo el centro del rect).
            if matches!(model.chart_view, crate::model::ChartView::Cielo) {
                if let Ok(guard) = model.carto_rect.lock() {
                    if let Some((rx, ry, rw, rh)) = *guard {
                        let z = model.wheel_zoom;
                        let z2 = (z * factor).clamp(0.25, 8.0);
                        let f = if z > 0.0 { z2 / z } else { 1.0 };
                        let rcx = rx + rw * 0.5;
                        let rcy = ry + rh * 0.5;
                        let (cx, cy) = cursor;
                        let pan_x = (cx - rcx) * (1.0 - f) + model.wheel_pan.0 * f;
                        let pan_y = (cy - rcy) * (1.0 - f) + model.wheel_pan.1 * f;
                        return Some(Msg::WheelSetView(z2, pan_x, pan_y));
                    }
                }
                return Some(Msg::WheelZoom(factor));
            }
            if matches!(model.chart_view, crate::model::ChartView::Carto) {
                if let Ok(guard) = model.carto_rect.lock() {
                    if let Some((rx, ry, rw, rh)) = *guard {
                        let base = (rw / 320.0).min(rh / 160.0);
                        let z = model.wheel_zoom;
                        let z2 = (z * factor).clamp(0.25, 8.0);
                        let s = base * z;
                        let s2 = base * z2;
                        if s > 0.0 && base > 0.0 {
                            let (cx, cy) = cursor;
                            let rcx = rx + rw * 0.5;
                            let rcy = ry + rh * 0.5;
                            let off_x = rcx - 320.0 * s / 2.0 + model.wheel_pan.0;
                            let off_y = rcy - 160.0 * s / 2.0 + model.wheel_pan.1;
                            let wx = (cx - off_x) / s;
                            let wy = (cy - off_y) / s;
                            let pan_x = cx - wx * s2 - rcx + 320.0 * s2 / 2.0;
                            let pan_y = cy - wy * s2 - rcy + 160.0 * s2 / 2.0;
                            return Some(Msg::WheelSetView(z2, pan_x, pan_y));
                        }
                    }
                }
            }
            Some(Msg::WheelZoom(factor))
        }
    }
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Cosmos>();
}

/// Proyecta un `DockItem` a un diente hospedado `(id, icono, etiqueta)` para
/// publicarlo en el rail de pata. El `id` codifica el `DockItem` (`to_u64`) y
/// vuelve tal cual en [`Msg::HostActivate`].
fn dock_item_tooth(item: model::DockItem) -> pata_host::HostedTooth {
    use model::{DockItem, ToolCat};
    let (icon, label): (&str, String) = match item {
        DockItem::Arbol => ("folder", "Biblioteca".to_string()),
        other => {
            let tc = other.tool_cat().unwrap_or(ToolCat::Principal);
            let icon = match tc {
                ToolCat::Astronomia => "astro",
                ToolCat::Sistema => "settings",
                _ => "tools",
            };
            (icon, tc.title().to_string())
        }
    };
    pata_host::HostedTooth::new(item.to_u64() as u32, icon, label)
}
