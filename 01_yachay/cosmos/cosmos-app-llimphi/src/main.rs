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
//! Pendiente: store de cartas + form de birth data; AstroCarto/box graphs/
//! research; scroll vertical en sidebar cuando hay muchos tiles.

use cosmos_canvas_llimphi::canvas_view;
use cosmos_engine::{
    combinaciones_de_carta, compose, corpus_inputs, Corpus, NatalOptions, PipelineRequest,
};
use cosmos_model::{
    Chart, ChartId, ChartKind, ContactId, StoredBirthData, StoredChartConfig, TimeCertainty,
};
use cosmos_render::{
    compose_wheel, AspectSummary, CompositionOpts, LayerKind, RenderModel, UranianGroup,
};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_button::{button_styled, ButtonPalette};
use llimphi_widget_tiled::{tiled_view_reorderable_cols, TileSpec, TiledPalette};
use wawa_config_llimphi::theme_from_wawa;

const WHEEL_SIZE: f32 = 720.0;
const SIDEBAR_WIDTH: f32 = 340.0;
const HARMONICS: &[u32] = &[1, 4, 5, 7, 9];

#[derive(Clone)]
enum Msg {
    WawaConfigChanged(Box<wawa_config::WawaConfig>),
    ToggleOverlay(OverlayKind),
    SetHarmonic(u32),
    SwapTiles(usize, usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum OverlayKind {
    Transit,
    Progression,
    SolarArc,
    Uranian,
    Lots,
    FixedStars,
    Midpoints,
}

impl OverlayKind {
    fn all() -> &'static [OverlayKind] {
        &[
            OverlayKind::Transit,
            OverlayKind::Progression,
            OverlayKind::SolarArc,
            OverlayKind::Uranian,
            OverlayKind::Lots,
            OverlayKind::FixedStars,
            OverlayKind::Midpoints,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            OverlayKind::Transit => "cosmos-overlay-transit",
            OverlayKind::Progression => "cosmos-overlay-progression",
            OverlayKind::SolarArc => "cosmos-overlay-solar-arc",
            OverlayKind::Uranian => "cosmos-overlay-uranian",
            OverlayKind::Lots => "cosmos-overlay-lots",
            OverlayKind::FixedStars => "cosmos-overlay-fixed-stars",
            OverlayKind::Midpoints => "cosmos-overlay-midpoints",
        }
    }

    fn to_request(self, target_age: f64) -> PipelineRequest {
        match self {
            OverlayKind::Transit => PipelineRequest::Transit,
            OverlayKind::Progression => PipelineRequest::SecondaryProgression {
                target_age_years: target_age,
            },
            OverlayKind::SolarArc => PipelineRequest::SolarArc {
                target_age_years: target_age,
            },
            OverlayKind::Uranian => PipelineRequest::Uranian,
            OverlayKind::Lots => PipelineRequest::Lots,
            OverlayKind::FixedStars => PipelineRequest::FixedStars,
            OverlayKind::Midpoints => PipelineRequest::Midpoints,
        }
    }
}

/// Identidad de cada tile del sidebar. El orden lo controla
/// `Model::panel_order`. Los tiles dinámicos (gated por overlay) entran o
/// salen del vec según `Model::overlays`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TileId {
    Carta,
    Modulos,
    Armonico,
    Cuerpos,
    Aspectos,
    BoxGraph,
    Corpus,
    Uraniano,
    CrossTransit,
    CrossProgression,
    CrossSolarArc,
}

impl TileId {
    fn label_key(self) -> &'static str {
        match self {
            TileId::Carta => "cosmos-tile-carta",
            TileId::Modulos => "cosmos-tile-modulos",
            TileId::Armonico => "cosmos-tile-armonico",
            TileId::Cuerpos => "cosmos-tile-cuerpos",
            TileId::Aspectos => "cosmos-tile-aspectos",
            TileId::BoxGraph => "cosmos-tile-box-graph",
            TileId::Corpus => "cosmos-tile-corpus",
            TileId::Uraniano => "cosmos-tile-uraniano",
            TileId::CrossTransit => "cosmos-tile-cross-transit",
            TileId::CrossProgression => "cosmos-tile-cross-progression",
            TileId::CrossSolarArc => "cosmos-tile-cross-solar-arc",
        }
    }
}

const DEFAULT_ORDER: &[TileId] = &[
    TileId::Carta,
    TileId::Modulos,
    TileId::Armonico,
    TileId::Cuerpos,
    TileId::Aspectos,
    TileId::BoxGraph,
    TileId::Corpus,
];

/// Corpus de interpretación embebido — la plantilla que viene con
/// `cosmos-corpus`. Se reemplaza más adelante por un loader que mire en
/// `~/.config/cosmos/corpus.ron` y caiga a este si no existe.
const CORPUS_DEFAULT_RON: &str = include_str!("../../cosmos-corpus/ejemplo.ron");

/// Devuelve el TileId dinámico que aporta un overlay, si aporta uno. Los
/// overlays que solo tienen efecto visual sobre el wheel (Lots, FixedStars,
/// Midpoints) no agregan tile propio — su toggle vive en el tile Módulos.
fn overlay_tile(kind: OverlayKind) -> Option<TileId> {
    match kind {
        OverlayKind::Uranian => Some(TileId::Uraniano),
        OverlayKind::Transit => Some(TileId::CrossTransit),
        OverlayKind::Progression => Some(TileId::CrossProgression),
        OverlayKind::SolarArc => Some(TileId::CrossSolarArc),
        OverlayKind::Lots | OverlayKind::FixedStars | OverlayKind::Midpoints => None,
    }
}

struct Model {
    chart: Chart,
    overlays: Vec<OverlayKind>,
    harmonic: u32,
    render: RenderModel,
    theme: Theme,
    error: Option<String>,
    /// Orden actual de tiles visibles. Se reordena por drag-to-swap; se
    /// extiende/recorta cuando los overlays con tile propio se prenden/apagan.
    panel_order: Vec<TileId>,
    corpus: Corpus,
    _wawa_watcher: Option<wawa_config::ConfigWatcher>,
}

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

        let chart = sample_chart();
        let overlays: Vec<OverlayKind> = Vec::new();
        let harmonic = 1;
        let (render, error) = compute(&chart, &overlays, harmonic);
        let corpus = Corpus::desde_ron(CORPUS_DEFAULT_RON).unwrap_or_default();
        Model {
            chart,
            overlays,
            harmonic,
            render,
            theme,
            error,
            panel_order: DEFAULT_ORDER.to_vec(),
            corpus,
            _wawa_watcher: watcher,
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
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
            }
            Msg::SetHarmonic(n) => {
                if m.harmonic != n {
                    m.harmonic = n;
                    let (render, error) = compute(&m.chart, &m.overlays, m.harmonic);
                    m.render = render;
                    m.error = error;
                }
            }
            Msg::SwapTiles(from, to) => {
                if from != to && from < m.panel_order.len() && to < m.panel_order.len() {
                    m.panel_order.swap(from, to);
                }
            }
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
            show_coord_labels: false,
            show_minor_aspects: false,
            dial_3d: true,
        };
        let commands = compose_wheel(&model.render, &opts);

        let canvas_bg = Color::from_rgba8(8, 10, 16, 255);
        let canvas = canvas_view::<Msg>(commands, WHEEL_SIZE, Some(canvas_bg));

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

// =====================================================================
// Barras top/bottom
// =====================================================================

fn header_bar(m: &RenderModel, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(
        rimay_localize::t_args(
            "cosmos-header",
            &[
                ("title", m.title.as_str().into()),
                ("asc", format!("{:.1}", m.ascendant_deg).into()),
                ("mc", format!("{:.1}", m.midheaven_deg).into()),
            ],
        ),
        12.0,
        theme.fg_text,
        Alignment::Start,
    )
}

fn status_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let txt = if let Some(err) = &model.error {
        rimay_localize::t_args("cosmos-status-error", &[("err", err.as_str().into())])
    } else {
        rimay_localize::t_args(
            "cosmos-status",
            &[
                ("ms", model.render.compute_ms.to_string().into()),
                ("layers", model.render.layers.len().to_string().into()),
                ("overlays", model.render.overlays.len().to_string().into()),
                ("aspects", model.render.aspect_summary.len().to_string().into()),
            ],
        )
    };
    let color = if model.error.is_some() {
        theme.fg_text
    } else {
        theme.fg_muted
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(txt, 11.0, color, Alignment::Start)
}

// =====================================================================
// Sidebar — un solo panel con tiled vertical drag-to-swap
// =====================================================================

fn side_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let palette = TiledPalette::from_theme(theme);
    let specs: Vec<TileSpec<Msg>> = model
        .panel_order
        .iter()
        .map(|tid| build_tile(*tid, model, theme))
        .collect();

    let tiled = tiled_view_reorderable_cols(
        specs,
        1,
        |from, to| {
            if from == to {
                None
            } else {
                Some(Msg::SwapTiles(from, to))
            }
        },
        &palette,
    );

    View::new(Style {
        size: Size {
            width: length(SIDEBAR_WIDTH),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![tiled])
}

fn build_tile(tid: TileId, model: &Model, theme: &Theme) -> TileSpec<Msg> {
    let label = rimay_localize::t(tid.label_key());
    let content = match tid {
        TileId::Carta => tile_carta(model, theme),
        TileId::Modulos => tile_modulos(model, theme),
        TileId::Armonico => tile_armonico(model, theme),
        TileId::Cuerpos => tile_cuerpos(&model.render, theme),
        TileId::Aspectos => tile_aspectos(&model.render, "natal", theme),
        TileId::BoxGraph => tile_box_graph(&model.render, theme),
        TileId::Corpus => tile_corpus(&model.render, &model.corpus, theme),
        TileId::Uraniano => tile_uraniano(&model.render.uranian_groups, theme),
        TileId::CrossTransit => tile_aspectos(&model.render, "transit", theme),
        TileId::CrossProgression => tile_aspectos(&model.render, "progression", theme),
        TileId::CrossSolarArc => tile_aspectos(&model.render, "solar_arc", theme),
    };
    TileSpec { label, content }
}

fn tile_container<I>(rows: I, theme: &Theme) -> View<Msg>
where
    I: IntoIterator<Item = View<Msg>>,
{
    let _ = theme;
    let children: Vec<View<Msg>> = rows.into_iter().collect();
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .clip(true)
    .children(children)
}

fn line(text: String, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(size + 4.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(text, size, color, Alignment::Start)
}

// ----- Carta -----

fn tile_carta(model: &Model, theme: &Theme) -> View<Msg> {
    let bd = &model.chart.birth_data;
    let lugar = bd
        .birthplace_label
        .clone()
        .unwrap_or_else(|| "(sin lugar)".into());
    let fecha = format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC{:+}",
        bd.year,
        bd.month,
        bd.day,
        bd.hour,
        bd.minute,
        bd.tz_offset_minutes as f32 / 60.0
    );
    let lat_long = format!(
        "{:.4}°{} · {:.4}°{}",
        bd.latitude_deg.abs(),
        if bd.latitude_deg >= 0.0 { "N" } else { "S" },
        bd.longitude_deg.abs(),
        if bd.longitude_deg >= 0.0 { "E" } else { "W" }
    );
    let angles = format!(
        "Asc {} · MC {} · Desc {} · IC {}",
        fmt_deg_sign(model.render.ascendant_deg),
        fmt_deg_sign(model.render.midheaven_deg),
        fmt_deg_sign(model.render.descendant_deg),
        fmt_deg_sign(model.render.imum_coeli_deg),
    );

    tile_container(
        vec![
            line(model.chart.label.clone(), 12.0, theme.fg_text),
            line(lugar, 10.0, theme.fg_muted),
            line(fecha, 10.0, theme.fg_muted),
            line(lat_long, 10.0, theme.fg_muted),
            line(angles, 10.0, theme.fg_text),
        ],
        theme,
    )
}

// ----- Módulos (toggles de overlays) -----

fn tile_modulos(model: &Model, theme: &Theme) -> View<Msg> {
    let pal_off = ButtonPalette::from_theme(theme);
    let pal_on = ButtonPalette {
        bg: theme.accent,
        bg_hover: theme.accent,
        fg: theme.bg_panel,
        radius: pal_off.radius,
    };

    let rows: Vec<View<Msg>> = OverlayKind::all()
        .iter()
        .map(|kind| {
            let active = model.overlays.contains(kind);
            let palette = if active { &pal_on } else { &pal_off };
            button_styled(
                rimay_localize::t(kind.label()),
                Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(22.0_f32),
                    },
                    flex_shrink: 0.0,
                    padding: Rect {
                        left: length(8.0_f32),
                        right: length(8.0_f32),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::Start),
                    ..Default::default()
                },
                Alignment::Start,
                palette,
                Msg::ToggleOverlay(*kind),
            )
        })
        .collect();

    tile_container(rows, theme)
}

// ----- Armónico (selector H1/H4/H5/H7/H9) -----

fn tile_armonico(model: &Model, theme: &Theme) -> View<Msg> {
    let pal_off = ButtonPalette::from_theme(theme);
    let pal_on = ButtonPalette {
        bg: theme.accent,
        bg_hover: theme.accent,
        fg: theme.bg_panel,
        radius: pal_off.radius,
    };

    let btns: Vec<View<Msg>> = HARMONICS
        .iter()
        .map(|h| {
            let active = model.harmonic == *h;
            let palette = if active { &pal_on } else { &pal_off };
            button_styled(
                format!("H{h}"),
                Style {
                    size: Size {
                        width: length(44.0_f32),
                        height: length(22.0_f32),
                    },
                    margin: Rect {
                        left: length(0.0_f32),
                        right: length(4.0_f32),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    padding: Rect {
                        left: length(0.0_f32),
                        right: length(0.0_f32),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::Center),
                    ..Default::default()
                },
                Alignment::Center,
                palette,
                Msg::SetHarmonic(*h),
            )
        })
        .collect();

    let row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(26.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(btns);

    tile_container(vec![row], theme)
}

// ----- Cuerpos -----

fn tile_cuerpos(render: &RenderModel, theme: &Theme) -> View<Msg> {
    let rows: Vec<View<Msg>> = render
        .layers
        .iter()
        .filter(|l| l.module_id == "natal" && matches!(l.kind, LayerKind::Bodies))
        .flat_map(|l| l.glyphs.iter())
        .map(|g| {
            let sign = signo_de_longitud(g.deg);
            let dms = fmt_dms(g.deg.rem_euclid(30.0) as f64);
            let body = simbolo_cuerpo(&g.symbol);
            let casa = g.house.map(|h| format!(" h{h}")).unwrap_or_default();
            let retro = if g.retrograde { " ℞" } else { "" };
            let dignity = g.dignity_marker.clone().unwrap_or_default();
            let line_str = format!("{body} {dms} {sign}{casa}{retro}{dignity}");
            line(line_str, 11.0, theme.fg_text)
        })
        .collect();
    tile_container(rows, theme)
}

// ----- Aspectos (filtrado por module_id) -----

fn tile_aspectos(render: &RenderModel, module_id: &str, theme: &Theme) -> View<Msg> {
    let mut asps: Vec<&AspectSummary> = render
        .aspect_summary
        .iter()
        .filter(|a| a.module_id == module_id)
        .collect();
    asps.sort_by(|a, b| a.orb_deg.partial_cmp(&b.orb_deg).unwrap_or(std::cmp::Ordering::Equal));
    let rows: Vec<View<Msg>> = asps
        .into_iter()
        .take(20)
        .map(|a| {
            let from = simbolo_cuerpo(&a.from_body);
            let to = simbolo_cuerpo(&a.to_body);
            let kind = simbolo_aspecto(&a.kind);
            let dms = fmt_dms(a.orb_deg);
            let dir = match a.applying {
                Some(true) => " ◂",
                Some(false) => " ▸",
                None => "",
            };
            line(format!("{from} {kind} {to}  {dms}{dir}"), 11.0, theme.fg_text)
        })
        .collect();
    if rows.is_empty() {
        return tile_container(
            vec![line(
                rimay_localize::t("cosmos-empty"),
                11.0,
                theme.fg_muted,
            )],
            theme,
        );
    }
    tile_container(rows, theme)
}

// ----- Uraniano (grupos del dial de 90°) -----

fn tile_uraniano(groups: &[UranianGroup], theme: &Theme) -> View<Msg> {
    if groups.is_empty() {
        return tile_container(
            vec![line(
                rimay_localize::t("cosmos-empty"),
                11.0,
                theme.fg_muted,
            )],
            theme,
        );
    }
    let rows: Vec<View<Msg>> = groups
        .iter()
        .take(16)
        .map(|g| {
            let bodies: Vec<String> = g.bodies.iter().map(|b| simbolo_cuerpo(b).into()).collect();
            line(
                format!("{:.1}°  {}", g.mod90_deg, bodies.join(" ")),
                11.0,
                theme.fg_text,
            )
        })
        .collect();
    tile_container(rows, theme)
}

// ----- Box graph (aspectarian triangular) -----

fn tile_box_graph(render: &RenderModel, theme: &Theme) -> View<Msg> {
    // 1. cuerpos natales en orden de longitud (estable porque la layer ya
    //    los emite en el orden canónico).
    let bodies: Vec<String> = render
        .layers
        .iter()
        .filter(|l| l.module_id == "natal" && matches!(l.kind, LayerKind::Bodies))
        .flat_map(|l| l.glyphs.iter())
        .map(|g| g.symbol.clone())
        .collect();
    if bodies.len() < 2 {
        return tile_container(
            vec![line(
                rimay_localize::t("cosmos-empty"),
                11.0,
                theme.fg_muted,
            )],
            theme,
        );
    }
    // 2. mapa (par ordenado) → símbolo de aspecto.
    let mut aspects: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();
    for a in &render.aspect_summary {
        if a.module_id != "natal" {
            continue;
        }
        let key = sorted_pair(&a.from_body, &a.to_body);
        aspects.insert(key, a.kind.clone());
    }
    // 3. filas triangulares: fila i = etiqueta cuerpo i + i celdas.
    const CELL: f32 = 22.0;
    const LBL: f32 = 24.0;
    let rows: Vec<View<Msg>> = bodies
        .iter()
        .enumerate()
        .map(|(i, body_i)| {
            let mut cells: Vec<View<Msg>> = Vec::with_capacity(i + 1);
            cells.push(box_cell(
                simbolo_cuerpo(body_i),
                theme.fg_text,
                None,
                LBL,
                CELL,
                theme,
            ));
            for body_j in bodies.iter().take(i) {
                let pair = sorted_pair(body_i, body_j);
                let asp = aspects.get(&pair);
                let (text, bg) = match asp {
                    Some(k) => (simbolo_aspecto(k), Some(theme.bg_panel_alt)),
                    None => ("·", None),
                };
                cells.push(box_cell(text, theme.fg_text, bg, CELL, CELL, theme));
            }
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: Dimension::auto(),
                    height: length(CELL),
                },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .children(cells)
        })
        .collect();
    tile_container(rows, theme)
}

fn box_cell(
    text: &'static str,
    fg: Color,
    bg: Option<Color>,
    w: f32,
    h: f32,
    _theme: &Theme,
) -> View<Msg> {
    let mut v = View::new(Style {
        size: Size {
            width: length(w),
            height: length(h),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(text.to_string(), 11.0, fg, Alignment::Center);
    if let Some(c) = bg {
        v = v.fill(c).radius(2.0);
    }
    v
}

fn sorted_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.into(), b.into())
    } else {
        (b.into(), a.into())
    }
}

// ----- Corpus (pasajes interpretativos) -----

fn tile_corpus(render: &RenderModel, corpus: &Corpus, theme: &Theme) -> View<Msg> {
    let (colocaciones, aspectos) = corpus_inputs(render);
    let combinaciones = combinaciones_de_carta(&colocaciones, &aspectos);
    let pasajes = corpus.interpretar(&combinaciones);
    let huecos = corpus.huecos(&combinaciones);

    let header_txt = rimay_localize::t_args(
        "cosmos-corpus-header",
        &[
            ("pasajes", pasajes.len().to_string().into()),
            ("huecos", huecos.len().to_string().into()),
            ("total", combinaciones.len().to_string().into()),
        ],
    );
    let mut rows: Vec<View<Msg>> = Vec::with_capacity(pasajes.len() * 2 + 1);
    rows.push(line(header_txt, 11.0, theme.fg_muted));

    if pasajes.is_empty() {
        rows.push(line(
            rimay_localize::t("cosmos-corpus-vacio"),
            11.0,
            theme.fg_muted,
        ));
    } else {
        for p in pasajes.iter().take(8) {
            rows.push(line(p.combinacion.to_string(), 10.0, theme.fg_muted));
            // Texto del pasaje: lo cortamos a ~140 chars para que la fila
            // siga siendo de una sola línea visible en el sidebar.
            let txt = recortar(&p.texto, 140);
            rows.push(line(txt, 11.0, theme.fg_text));
        }
    }
    tile_container(rows, theme)
}

fn recortar(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            return out;
        }
        out.push(ch);
    }
    out
}

// =====================================================================
// Compose
// =====================================================================

fn sample_chart() -> Chart {
    Chart {
        id: ChartId::new(),
        contact_id: ContactId::new(),
        kind: ChartKind::Natal,
        label: rimay_localize::t("cosmos-demo-title"),
        birth_data: StoredBirthData {
            year: 1990,
            month: 6,
            day: 21,
            hour: 12,
            minute: 0,
            second: 0.0,
            tz_offset_minutes: -300,
            latitude_deg: -12.0464,
            longitude_deg: -77.0428,
            altitude_m: 154.0,
            time_certainty: TimeCertainty::Estimated,
            subject_name: None,
            birthplace_label: Some("Lima".into()),
        },
        config: StoredChartConfig::default(),
        related_chart_id: None,
        created_at_ms: 0,
    }
}

fn compute(
    chart: &Chart,
    overlays: &[OverlayKind],
    harmonic: u32,
) -> (RenderModel, Option<String>) {
    let target_age = 35.0;
    let requests: Vec<PipelineRequest> = overlays
        .iter()
        .map(|k| k.to_request(target_age))
        .collect();
    let opts = NatalOptions {
        show_majors: true,
        show_minors: false,
        orb_multiplier: 1.0,
        show_dignities: true,
        harmonic,
    };
    match cosmos_engine::compose_with_options(chart, 0, &requests, &opts) {
        Ok(r) => (r, None),
        Err(e) => {
            let msg = format!("{e}");
            (
                compose(chart, 0, &[]).unwrap_or_else(|_| cosmos_engine::compute_mock(chart)),
                Some(msg),
            )
        }
    }
}

// =====================================================================
// Helpers de formateo
// =====================================================================

const SIGNOS: [&str; 12] = [
    "♈", "♉", "♊", "♋", "♌", "♍", "♎", "♏", "♐", "♑", "♒", "♓",
];

fn signo_de_longitud(deg: f32) -> &'static str {
    SIGNOS[((deg.rem_euclid(360.0) / 30.0) as usize) % 12]
}

fn fmt_deg_sign(deg: f32) -> String {
    let dms = fmt_dms((deg.rem_euclid(30.0)) as f64);
    format!("{dms} {}", signo_de_longitud(deg))
}

fn fmt_dms(deg: f64) -> String {
    let total_min = (deg.abs() * 60.0).round() as i64;
    let d = total_min / 60;
    let m = total_min % 60;
    format!("{:>2}°{:02}'", d, m)
}

fn simbolo_cuerpo(s: &str) -> &'static str {
    match s {
        "sun" => "☉",
        "moon" => "☽",
        "mercury" => "☿",
        "venus" => "♀",
        "mars" => "♂",
        "jupiter" => "♃",
        "saturn" => "♄",
        "uranus" => "♅",
        "neptune" => "♆",
        "pluto" => "♇",
        "earth" => "⊕",
        "north_node" | "ascending_node" => "☊",
        "south_node" | "descending_node" => "☋",
        "lilith" => "⚸",
        "chiron" => "⚷",
        "mean_node" => "☊",
        "asc" => "Asc",
        "desc" => "Desc",
        "mc" => "MC",
        "ic" => "IC",
        _ => "·",
    }
}

fn simbolo_aspecto(s: &str) -> &'static str {
    match s {
        "conjunction" => "☌",
        "opposition" => "☍",
        "trine" => "△",
        "square" => "□",
        "sextile" => "⚹",
        "quincunx" => "⚻",
        "semi_sextile" => "⚺",
        "semi_square" => "∠",
        "sesquiquadrate" => "⚼",
        _ => "·",
    }
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Cosmos>();
}
