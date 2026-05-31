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
mod engine;
mod format;
mod library;
mod model;
mod persist;
mod tools;
mod view;

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
    if m.open.is_empty() {
        let (render, error) = compute(&m.chart, &m.overlays, m.harmonic, m.cfg.minor_aspects);
        m.render = render;
        m.error = error;
        return;
    }
    let overlays = m.overlays.clone();
    let (h, minor) = (m.harmonic, m.cfg.minor_aspects);
    let active = m.active_tab.min(m.open.len() - 1);
    for i in 0..m.open.len() {
        let (render, error) = compute(&m.open[i].chart, &overlays, h, minor);
        m.open[i].render = render;
        if i == active {
            m.render = m.open[i].render.clone();
            m.error = error;
        }
    }
}

/// Render puntual de una carta con las opciones globales actuales.
fn compute_render(m: &Model, chart: &cosmos_model::Chart) -> cosmos_render::RenderModel {
    compute(chart, &m.overlays, m.harmonic, m.cfg.minor_aspects).0
}

fn recompute_astro(m: &mut Model) {
    m.astro = compute_astro(&m.chart, m.cfg.use_now);
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

fn new_contact(m: &mut Model) {
    let Some(store) = &m.store else { return };
    // Grupo destino: el seleccionado, o el grupo del contacto/carta
    // seleccionados; si nada, raíz.
    let group = m.selected_node().and_then(|n| match n.kind {
        library::NavKind::Group => library::parse_group_key(&n.key),
        _ => n.parent.as_deref().and_then(library::parse_group_key),
    });
    match store.create_contact(group, "Contacto nuevo", None) {
        Ok(c) => {
            if let Some(g) = group {
                m.nav_expanded.insert(format!("g:{g}"));
            }
            refresh_nav(m);
            start_rename(m, format!("c:{}", c.id));
        }
        Err(e) => m.error = Some(format!("crear contacto: {e}")),
    }
}

fn new_chart(m: &mut Model) {
    let Some(store) = &m.store else { return };
    // Contacto destino: el seleccionado, o el contacto padre de la carta
    // seleccionada.
    let contact = m.selected_node().and_then(|n| match n.kind {
        library::NavKind::Contact => library::parse_contact_key(&n.key),
        library::NavKind::Chart => n.parent.as_deref().and_then(library::parse_contact_key),
        library::NavKind::Group => None,
    });
    let Some(contact) = contact else {
        m.error = Some("seleccioná un contacto para crear la carta".into());
        return;
    };
    // Clona los datos de nacimiento/config de la carta actual.
    let res = store.create_chart(
        contact,
        cosmos_model::ChartKind::Natal,
        "Carta nueva",
        &m.chart.birth_data,
        &m.chart.config,
        None,
    );
    match res {
        Ok(ch) => {
            m.nav_expanded.insert(format!("c:{contact}"));
            refresh_nav(m);
            start_rename(m, format!("h:{}", ch.id));
        }
        Err(e) => m.error = Some(format!("crear carta: {e}")),
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

fn set_theme_dark(m: &mut Model, dark: bool) {
    m.cfg.theme_dark = dark;
    m.theme = if dark { Theme::dark() } else { Theme::light() };
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
        MenuCmd::Theme(dark) => set_theme_dark(m, dark),
        MenuCmd::Duplicar => do_duplicar(m),
        MenuCmd::Recargar => do_recargar(m),
        MenuCmd::Eliminar => do_eliminar(m),
        MenuCmd::SetChartView(cv) => m.chart_view = cv,
        MenuCmd::GoToolCat(tc) => m.tool_cat = tc,
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
    });
}

impl App for Cosmos {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "cosmos · canvas (llimphi)"
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
        let base = if ui.cfg.theme_dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let theme = theme_from_wawa(&cfg_wawa, &base);
        let (render, error) = compute(&chart, &ui.overlays, ui.harmonic, ui.cfg.minor_aspects);
        let astro = compute_astro(&chart, ui.cfg.use_now);
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

        Model {
            chart,
            overlays: ui.overlays,
            harmonic: ui.harmonic,
            render,
            astro,
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
            nav_w: ui.nav_w,
            tools_w: ui.tools_w,
            nav_open: ui.nav_open,
            tools_open: ui.tools_open,
            chart_view: ui.chart_view,
            tool_cat: ui.tool_cat,
            expanded_panels: ui.expanded_panels,
            menu_open: None,
            ctx_open: None,
            _wawa_watcher: watcher,
            _chart_watcher: chart_watcher,
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        let mut persist = false;
        // Cualquier interacción que no sea abrir un menú limpia la nota
        // efímera de estado.
        match &msg {
            Msg::OpenMenu(_) | Msg::WawaConfigChanged(_) => {}
            _ => m.status_note = None,
        }
        match msg {
            Msg::WawaConfigChanged(cfg) => {
                m.theme = theme_from_wawa(&cfg, &m.theme);
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
            // navegación
            Msg::ToggleNavNode(key) => m.toggle_nav(key),
            Msg::NavClick(key) => nav_click(&mut m, key),
            Msg::NewGroup => new_group(&mut m),
            Msg::NewContact => new_contact(&mut m),
            Msg::NewChart => new_chart(&mut m),
            Msg::DeleteSelected => delete_selected(&mut m),
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
            Msg::SetThemeDark(dark) => {
                set_theme_dark(&mut m, dark);
                persist = true;
            }
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
                m.ctx_open = None;
            }
            Msg::MenuPick(kind, idx) => {
                m.menu_open = None;
                let cmd = chrome::menu_entries(kind, &m).get(idx).map(|e| e.cmd);
                if let Some(cmd) = cmd {
                    apply_cmd(&mut m, cmd);
                    persist = true;
                }
            }
            Msg::CloseMenu => m.menu_open = None,
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
            Msg::CloseCtx => m.ctx_open = None,
            // layout guardable
            Msg::SetNavWidth(dx) => m.nudge_nav(dx),
            Msg::SetToolsWidth(dx) => m.nudge_tools(dx),
            Msg::PersistLayout => persist = true,
            // panel de herramientas
            Msg::SelectToolCat(c) => {
                m.tool_cat = c;
                persist = true;
            }
            Msg::ToggleToolPanel(p) => {
                m.toggle_panel(p);
                persist = true;
            }
            // tipo de gráfica
            Msg::SetChartView(v) => {
                m.chart_view = v;
                persist = true;
            }
        }
        if persist {
            save_ui(&m);
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let menu = chrome::menu_bar(model, &theme);
        let status = chrome::status_bar(model, &theme);
        let sp = SplitterPalette::from_theme(&theme);

        let center = chrome::center_view(model, &theme);

        // Zona derecha: centro (flex) + panel de herramientas (fijo,
        // resizable). Arrastrar el divisor a la derecha achica las
        // herramientas (ver Model::nudge_tools).
        let center_and_tools = if model.tools_open {
            splitter_two(
                Direction::Row,
                center,
                PaneSize::Flex,
                tools::tools_panel(model, &theme),
                PaneSize::Fixed(model.tools_w),
                |phase, dx| match phase {
                    DragPhase::Move => Some(Msg::SetToolsWidth(dx)),
                    DragPhase::End => Some(Msg::PersistLayout),
                },
                &sp,
            )
        } else {
            center
        };

        // Zona completa: árbol de datos (fijo, resizable) + lo anterior.
        let body = if model.nav_open {
            splitter_two(
                Direction::Row,
                chrome::nav_tree(model, &theme),
                PaneSize::Fixed(model.nav_w),
                center_and_tools,
                PaneSize::Flex,
                |phase, dx| match phase {
                    DragPhase::Move => Some(Msg::SetNavWidth(dx)),
                    DragPhase::End => Some(Msg::PersistLayout),
                },
                &sp,
            )
        } else {
            center_and_tools
        };

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
        chrome::overlay_view(model, &model.theme)
    }

    fn on_key(model: &Model, ev: &llimphi_ui::KeyEvent) -> Option<Msg> {
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
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Cosmos>();
}
