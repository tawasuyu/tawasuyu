//! `cosmos-app-llimphi` — visor del lienzo astrológico sobre Llimphi.
//!
//! Llama a [`cosmos_engine::compose`] con un `Chart` sample (sin store
//! todavía) y pinta el `RenderModel` resultante con `cosmos-canvas-llimphi`.
//! Eternal-bridge prendido por default → cuerpos calculados con VSOP2013,
//! casas Placidus, aspectos mayores.
//!
//! Layout: wheel al centro, sidebar derecha con tiles draggables (uno por
//! módulo). Cada tile aporta su slice de la UI — toggles, listas, controles
//! propios. El usuario arrastra la title bar de un tile sobre otro y los
//! intercambia (drag-to-swap vía llimphi-widget-tiled).
//!
//! El crate está partido en módulos: `model` (Model+Msg+OverlayKind+TileId),
//! `persist` (UiState/ChartFile + IO de cartas/ui + watcher), `engine`
//! (sample_chart + compute), `format` (símbolos/fmt), `astrocarto` (tile del
//! mapa equirectangular) y `view` (chrome + tiles). Acá queda el `impl App`.

mod astrocarto;
mod engine;
mod format;
mod model;
mod persist;
mod view;

use cosmos_engine::Corpus;
use cosmos_render::{compose_wheel_with_hits, CompositionOpts};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    length, percent, Dimension, FlexDirection, Size, Style,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, View};
use wawa_config_llimphi::theme_from_wawa;

use cosmos_canvas_llimphi::canvas_view_clickable;

use crate::engine::{compute, sample_chart};
use crate::model::{overlay_tile, Model, Msg, WHEEL_SIZE};
use crate::persist::{
    generate_card_name, load_card, load_chart_from_disk, load_ui_state, save_card,
    save_chart_to_disk, save_ui_state, spawn_chart_watcher, UiState,
};
use crate::view::{header_bar, side_panel, status_bar};

/// Corpus de interpretación embebido — la plantilla que viene con
/// `cosmos-corpus`. Se reemplaza más adelante por un loader que mire en
/// `~/.config/cosmos/corpus.ron` y caiga a este si no existe.
const CORPUS_DEFAULT_RON: &str = include_str!("../../cosmos-corpus/ejemplo.ron");

struct Cosmos;

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
        let cfg = wawa_config::WawaConfig::load();
        let theme = theme_from_wawa(&cfg, &Theme::dark());
        let _ = rimay_localize::set_locale(&cfg.lang);
        let handle_clone = handle.clone();
        let watcher = wawa_config::ConfigWatcher::spawn(move |new_cfg| {
            handle_clone.dispatch(Msg::WawaConfigChanged(Box::new(new_cfg)));
        })
        .map_err(|e| eprintln!("cosmos · wawa-config watcher: {e}"))
        .ok();

        let chart = load_chart_from_disk().unwrap_or_else(|| {
            let c = sample_chart();
            // Sembramos el archivo en el primer arranque para que el
            // usuario pueda editar la sample con su editor.
            save_chart_to_disk(&c);
            c
        });
        let ui = load_ui_state();
        let (render, error) = compute(&chart, &ui.overlays, ui.harmonic);
        let corpus = Corpus::desde_ron(CORPUS_DEFAULT_RON).unwrap_or_default();
        let chart_watcher = spawn_chart_watcher(handle);
        Model {
            chart,
            overlays: ui.overlays,
            harmonic: ui.harmonic,
            render,
            theme,
            error,
            panel_order: ui.panel_order,
            corpus,
            selected_body: None,
            _wawa_watcher: watcher,
            _chart_watcher: chart_watcher,
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        let mut persist = false;
        match msg {
            Msg::WawaConfigChanged(cfg) => {
                m.theme = theme_from_wawa(&cfg, &m.theme);
                if cfg.lang != rimay_localize::current_locale() {
                    let _ = rimay_localize::set_locale(&cfg.lang);
                }
            }
            Msg::ToggleOverlay(kind) => {
                let now_active = if let Some(idx) = m.overlays.iter().position(|k| *k == kind) {
                    m.overlays.remove(idx);
                    false
                } else {
                    m.overlays.push(kind);
                    true
                };
                if let Some(tile) = overlay_tile(kind) {
                    if now_active {
                        if !m.panel_order.contains(&tile) {
                            m.panel_order.push(tile);
                        }
                    } else {
                        m.panel_order.retain(|t| *t != tile);
                    }
                }
                let (render, error) = compute(&m.chart, &m.overlays, m.harmonic);
                m.render = render;
                m.error = error;
                persist = true;
            }
            Msg::SetHarmonic(n) => {
                if m.harmonic != n {
                    m.harmonic = n;
                    let (render, error) = compute(&m.chart, &m.overlays, m.harmonic);
                    m.render = render;
                    m.error = error;
                    persist = true;
                }
            }
            Msg::SwapTiles(from, to) => {
                if from != to && from < m.panel_order.len() && to < m.panel_order.len() {
                    m.panel_order.swap(from, to);
                    persist = true;
                }
            }
            Msg::ChartFileChanged => {
                if let Some(new_chart) = load_chart_from_disk() {
                    m.chart = new_chart;
                    let (render, error) = compute(&m.chart, &m.overlays, m.harmonic);
                    m.render = render;
                    m.error = error;
                }
            }
            Msg::CargarCarta(name) => {
                if let Some(loaded) = load_card(&name) {
                    m.chart = loaded;
                    save_chart_to_disk(&m.chart);
                    let (render, error) = compute(&m.chart, &m.overlays, m.harmonic);
                    m.render = render;
                    m.error = error;
                } else {
                    m.error = Some(format!("no se pudo cargar carta: {name}"));
                }
            }
            Msg::DuplicarActual => {
                let name = generate_card_name(&m.chart);
                save_card(&name, &m.chart);
            }
            Msg::SelectBody(sel) => {
                // Toggle: si se clickeó el mismo cuerpo, deselecciona.
                m.selected_body = if m.selected_body == sel { None } else { sel };
            }
        }
        if persist {
            save_ui_state(&UiState {
                panel_order: m.panel_order.clone(),
                overlays: m.overlays.clone(),
                harmonic: m.harmonic,
            });
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;

        let opts = CompositionOpts {
            size: WHEEL_SIZE,
            rot_offset_deg: 0.0,
            include_bodies: true,
            palette: cosmos_render::Palette::dark(),
            draw_ascensional_cross: true,
            // Coord labels visibles + dedupe por DD°MM'<Sg>: el usuario
            // ve la coordenada exacta de cada planeta y cada cusp de
            // casa, sin que se pise una posición duplicada.
            show_coord_labels: true,
            show_minor_aspects: false,
            dial_3d: true,
            selected_body: model.selected_body.clone(),
        };
        let (commands, hits) = compose_wheel_with_hits(&model.render, &opts);

        let canvas_bg = Color::from_rgba8(8, 10, 16, 255);
        // Click en un planeta natal → selecciona; click en vacío →
        // deselecciona. `hits` se mueve dentro del closure y vive
        // hasta el próximo render.
        let canvas = canvas_view_clickable::<Msg, _>(
            commands,
            WHEEL_SIZE,
            Some(canvas_bg),
            move |wx, wy| {
                let picked: Option<String> = hits.pick(wx, wy).map(str::to_string);
                Some(Msg::SelectBody(picked))
            },
        );

        let header = header_bar(&model.render, &theme);
        let status = status_bar(model, &theme);
        let sidebar = side_panel(model, &theme);

        let body = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            min_size: Size {
                width: length(0.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![canvas, sidebar]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, body, status])
    }
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Cosmos>();
}
