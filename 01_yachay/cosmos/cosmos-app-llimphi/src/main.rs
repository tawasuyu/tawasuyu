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
mod model;
mod persist;
mod view;

use cosmos_engine::Corpus;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, FlexDirection, Size, Style};
use llimphi_ui::{App, Handle, Key, KeyState, NamedKey, View};
use wawa_config_llimphi::theme_from_wawa;

use crate::astroview::compute_astro;
use crate::chrome::MenuCmd;
use crate::engine::{compute, sample_chart};
use crate::model::{Model, Msg, ViewKind, WheelOpt};
use crate::persist::{
    delete_card, generate_card_name, load_card, load_chart_from_disk, load_ui_state, save_card,
    save_chart_to_disk, save_ui_state, spawn_chart_watcher, UiState,
};

const CORPUS_DEFAULT_RON: &str = include_str!("../../cosmos-corpus/ejemplo.ron");

struct Cosmos;

// =====================================================================
// Helpers de transición (reusados por mensajes directos y menú)
// =====================================================================

fn recompute_chart(m: &mut Model) {
    let (render, error) = compute(&m.chart, &m.overlays, m.harmonic, m.cfg.minor_aspects);
    m.render = render;
    m.error = error;
}

fn recompute_astro(m: &mut Model) {
    m.astro = compute_astro(&m.chart, m.cfg.use_now);
}

fn open_view(m: &mut Model, v: ViewKind) {
    if let Some(i) = m.tabs.iter().position(|t| *t == v) {
        m.active_tab = i;
    } else {
        m.tabs.push(v);
        m.active_tab = m.tabs.len() - 1;
    }
}

fn close_tab(m: &mut Model, i: usize) {
    if i >= m.tabs.len() {
        return;
    }
    m.tabs.remove(i);
    if m.tabs.is_empty() {
        m.tabs.push(ViewKind::Rueda);
        m.active_tab = 0;
        return;
    }
    if m.active_tab > i {
        m.active_tab -= 1;
    } else if m.active_tab >= m.tabs.len() {
        m.active_tab = m.tabs.len() - 1;
    }
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

fn do_cargar(m: &mut Model, name: String) {
    if let Some(loaded) = load_card(&name) {
        m.chart = loaded;
        m.selected_card = Some(name);
        save_chart_to_disk(&m.chart);
        recompute_chart(m);
        recompute_astro(m);
    } else {
        m.error = Some(format!("no se pudo cargar carta: {name}"));
    }
}

fn do_nueva(m: &mut Model) {
    let c = sample_chart();
    save_chart_to_disk(&c);
    m.chart = c;
    m.selected_card = None;
    recompute_chart(m);
    recompute_astro(m);
    m.status_note = Some("Carta de ejemplo cargada".into());
}

fn do_duplicar(m: &mut Model) {
    let name = generate_card_name(&m.chart);
    save_card(&name, &m.chart);
    m.selected_card = Some(name.clone());
    m.status_note = Some(format!("Carta duplicada: {name}"));
}

fn do_recargar(m: &mut Model) {
    if let Some(c) = load_chart_from_disk() {
        m.chart = c;
        recompute_chart(m);
        recompute_astro(m);
        m.status_note = Some("Carta recargada de disco".into());
    }
}

fn do_eliminar(m: &mut Model) {
    if let Some(name) = m.selected_card.clone() {
        delete_card(&name);
        m.selected_card = None;
        m.status_note = Some(format!("Carta eliminada: {name}"));
    }
}

fn apply_cmd(m: &mut Model, cmd: MenuCmd) {
    match cmd {
        MenuCmd::Sep => {}
        MenuCmd::Nueva => do_nueva(m),
        MenuCmd::Duplicar => do_duplicar(m),
        MenuCmd::Recargar => do_recargar(m),
        MenuCmd::Eliminar => do_eliminar(m),
        MenuCmd::Open(v) => open_view(m, v),
        MenuCmd::CerrarTab => close_tab(m, m.active_tab),
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
        tabs: m.tabs.clone(),
        active_tab: m.active_tab,
        cfg: m.cfg.clone(),
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
            tabs: ui.tabs,
            active_tab: ui.active_tab,
            selected_card: None,
            selected_body: None,
            exp_cartas: true,
            exp_astrologia: true,
            exp_astronomia: true,
            exp_sistema: false,
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
            // navegación
            Msg::SelectView(v) => {
                open_view(&mut m, v);
                persist = true;
            }
            Msg::ActivateTab(i) => {
                if i < m.tabs.len() {
                    m.active_tab = i;
                    persist = true;
                }
            }
            Msg::CloseTab(i) => {
                close_tab(&mut m, i);
                persist = true;
            }
            Msg::ToggleNavGroup(g) => m.toggle_group(g),
            Msg::CargarCarta(name) => do_cargar(&mut m, name),
            Msg::ChartFileChanged => {
                if let Some(c) = load_chart_from_disk() {
                    m.chart = c;
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
                m.cfg.theme_dark = dark;
                m.theme = if dark { Theme::dark() } else { Theme::light() };
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
        }
        if persist {
            save_ui(&m);
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let menu = chrome::menu_bar(model, &theme);
        let nav = chrome::nav_tree(model, &theme);
        let tabs = chrome::tab_area(model, &theme);
        let status = chrome::status_bar(model, &theme);

        let tab_box = View::new(Style {
            flex_grow: 1.0,
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(0.0_f32),
                height: percent(1.0_f32),
            },
            min_size: Size {
                width: llimphi_ui::llimphi_layout::taffy::prelude::length(0.0_f32),
                height: llimphi_ui::llimphi_layout::taffy::prelude::length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![tabs]);

        let body = View::new(Style {
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
        .children(vec![nav, tab_box]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![menu, body, status])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        chrome::overlay_view(model, &model.theme)
    }

    fn on_key(model: &Model, ev: &llimphi_ui::KeyEvent) -> Option<Msg> {
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
                Some(Msg::CloseTab(model.active_tab))
            }
            _ => None,
        }
    }
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Cosmos>();
}
