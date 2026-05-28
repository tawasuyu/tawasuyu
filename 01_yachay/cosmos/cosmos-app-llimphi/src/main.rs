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
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
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
    /// El archivo `cosmos-chart.json` cambió en disco — recargá.
    ChartFileChanged,
    /// Cargar una carta guardada de `cosmos-charts/<name>.json`.
    CargarCarta(String),
    /// Duplicar la carta actual a un nuevo archivo en `cosmos-charts/`.
    DuplicarActual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TileId {
    Carta,
    Modulos,
    Armonico,
    Cuerpos,
    Aspectos,
    BoxGraph,
    Cualidades,
    AstroCarto,
    Cartas,
    Corpus,
    Uraniano,
    Lotes,
    EstrellasFijas,
    PuntosMedios,
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
            TileId::Cualidades => "cosmos-tile-cualidades",
            TileId::AstroCarto => "cosmos-tile-astrocarto",
            TileId::Cartas => "cosmos-tile-cartas",
            TileId::Corpus => "cosmos-tile-corpus",
            TileId::Uraniano => "cosmos-tile-uraniano",
            TileId::Lotes => "cosmos-tile-lotes",
            TileId::EstrellasFijas => "cosmos-tile-estrellas-fijas",
            TileId::PuntosMedios => "cosmos-tile-puntos-medios",
            TileId::CrossTransit => "cosmos-tile-cross-transit",
            TileId::CrossProgression => "cosmos-tile-cross-progression",
            TileId::CrossSolarArc => "cosmos-tile-cross-solar-arc",
        }
    }
}

const DEFAULT_ORDER: &[TileId] = &[
    TileId::Carta,
    TileId::Cartas,
    TileId::Modulos,
    TileId::Armonico,
    TileId::Cuerpos,
    TileId::Aspectos,
    TileId::BoxGraph,
    TileId::Cualidades,
    TileId::AstroCarto,
    TileId::Corpus,
];

/// Subdirectorio dentro del config dir donde viven las cartas guardadas
/// como archivos individuales `<nombre>.json`. El usuario lo gestiona
/// con su file manager — la app solo lista, lee y escribe.
const CHARTS_SUBDIR: &str = "cosmos-charts";

/// Corpus de interpretación embebido — la plantilla que viene con
/// `cosmos-corpus`. Se reemplaza más adelante por un loader que mire en
/// `~/.config/cosmos/corpus.ron` y caiga a este si no existe.
const CORPUS_DEFAULT_RON: &str = include_str!("../../cosmos-corpus/ejemplo.ron");

/// Nombre del archivo JSON donde persiste el estado de la UI (orden de
/// tiles + overlays activos + armónico). Vive en el config dir de wawa
/// para no acoplar a un dirs propio por app — un solo árbol de config.
const UI_STATE_FILE: &str = "cosmos-ui.json";

/// Nombre del archivo JSON donde persiste la carta cargada. El usuario
/// edita ESTE archivo con su editor para cambiar fecha/lat/long/label;
/// la app reacciona vía watcher (mismo patrón que wawa-config). Sin
/// form de UI hasta que llegue la fase de store de cartas.
const CHART_FILE: &str = "cosmos-chart.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiState {
    #[serde(default)]
    panel_order: Vec<TileId>,
    #[serde(default)]
    overlays: Vec<OverlayKind>,
    #[serde(default = "default_harmonic")]
    harmonic: u32,
}

fn default_harmonic() -> u32 {
    1
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            panel_order: DEFAULT_ORDER.to_vec(),
            overlays: Vec::new(),
            harmonic: 1,
        }
    }
}

fn ui_state_path() -> Option<PathBuf> {
    wawa_config::config_dir().map(|d| d.join(UI_STATE_FILE))
}

fn chart_path() -> Option<PathBuf> {
    wawa_config::config_dir().map(|d| d.join(CHART_FILE))
}

/// Forma serializada minimal de un Chart natal. Pierde `id`/`contact_id`
/// (se regeneran al cargar) y `created_at_ms` — son metadata interna que
/// no aporta al usuario que edita el JSON a mano.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChartFile {
    label: String,
    birth_data: StoredBirthData,
    #[serde(default)]
    config: StoredChartConfig,
}

impl From<&Chart> for ChartFile {
    fn from(c: &Chart) -> Self {
        Self {
            label: c.label.clone(),
            birth_data: c.birth_data.clone(),
            config: c.config.clone(),
        }
    }
}

impl ChartFile {
    fn into_chart(self) -> Chart {
        Chart {
            id: ChartId::new(),
            contact_id: ContactId::new(),
            kind: ChartKind::Natal,
            label: self.label,
            birth_data: self.birth_data,
            config: self.config,
            related_chart_id: None,
            created_at_ms: 0,
        }
    }
}

fn load_chart_from_disk() -> Option<Chart> {
    let path = chart_path()?;
    let bytes = std::fs::read(&path).ok()?;
    let f: ChartFile = serde_json::from_slice(&bytes)
        .map_err(|e| eprintln!("cosmos · chart-file: no se pudo parsear {path:?}: {e}"))
        .ok()?;
    Some(f.into_chart())
}

/// Arranca un watcher sobre `cosmos-chart.json` que dispara
/// `Msg::ChartFileChanged` al detectar `Modify`/`Create` en el archivo.
/// Devuelve `None` si no hay config dir disponible o si notify falla —
/// la app sigue funcionando sin hot-reload, solo no reaccionará a edits
/// externos hasta el próximo arranque.
fn spawn_chart_watcher(handle: &Handle<Msg>) -> Option<notify::RecommendedWatcher> {
    let path = chart_path()?;
    // Asegurá que el dir existe y el archivo está sembrado antes de
    // watchearlo — notify exige que el path exista al `watch`.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if !path.exists() {
        // Sembrado lazy: si nunca pasó por init(), no hay archivo. Lo
        // creamos vacío para que el watcher tenga algo que mirar; init
        // lo sobreescribirá con el sample al arrancar.
        let _ = std::fs::write(&path, b"{}");
    }
    let h = handle.clone();
    wawa_config::watch_path(&path, move |ev: notify::Event| {
        use notify::EventKind;
        if matches!(
            ev.kind,
            EventKind::Modify(_) | EventKind::Create(_)
        ) {
            h.dispatch(Msg::ChartFileChanged);
        }
    })
    .map_err(|e| eprintln!("cosmos · chart-watcher: {e}"))
    .ok()
}

// =====================================================================
// Store de cartas (multi-archivo)
// =====================================================================

fn charts_dir() -> Option<PathBuf> {
    wawa_config::config_dir().map(|d| d.join(CHARTS_SUBDIR))
}

/// Lista los nombres de las cartas guardadas (sin `.json`), ordenados
/// alfabéticamente. Lee el directorio en cada call — barato porque son
/// pocos archivos y la app no es hot-path.
fn list_cards() -> Vec<String> {
    let Some(dir) = charts_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    out.push(stem.to_string());
                }
            }
        }
    }
    out.sort();
    out
}

fn load_card(name: &str) -> Option<Chart> {
    let path = charts_dir()?.join(format!("{name}.json"));
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice::<ChartFile>(&bytes)
        .map_err(|e| eprintln!("cosmos · load_card({name}): {e}"))
        .ok()
        .map(|f| f.into_chart())
}

fn save_card(name: &str, chart: &Chart) {
    let Some(dir) = charts_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{name}.json"));
    let f: ChartFile = chart.into();
    if let Ok(json) = serde_json::to_vec_pretty(&f) {
        if let Err(e) = std::fs::write(&path, json) {
            eprintln!("cosmos · save_card({name}): {e}");
        }
    }
}

/// Genera un nombre de archivo único para `chart` — sluggea el label,
/// concatena fecha de nacimiento, y agrega un sufijo numérico si hubiera
/// colisión con un archivo ya existente.
fn generate_card_name(chart: &Chart) -> String {
    let bd = &chart.birth_data;
    let slug: String = chart
        .label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = format!("{slug}-{:04}-{:02}-{:02}", bd.year, bd.month, bd.day);
    let mut name = base.clone();
    let mut i = 2;
    while let Some(dir) = charts_dir() {
        if !dir.join(format!("{name}.json")).exists() {
            break;
        }
        name = format!("{base}-{i}");
        i += 1;
        if i > 99 {
            break;
        }
    }
    name
}

fn save_chart_to_disk(chart: &Chart) {
    let Some(path) = chart_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let f: ChartFile = chart.into();
    if let Ok(json) = serde_json::to_vec_pretty(&f) {
        if let Err(e) = std::fs::write(&path, json) {
            eprintln!("cosmos · chart-file: write fallido {path:?}: {e}");
        }
    }
}

fn load_ui_state() -> UiState {
    let Some(path) = ui_state_path() else {
        return UiState::default();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return UiState::default();
    };
    match serde_json::from_slice::<UiState>(&bytes) {
        Ok(mut s) => {
            // Garantía: las always-on tienen que estar. Si el archivo viejo
            // tiene un order corto o le falta alguna, completamos al final
            // sin perder el orden que el usuario ya eligió.
            for tid in DEFAULT_ORDER {
                if !s.panel_order.contains(tid) {
                    s.panel_order.push(*tid);
                }
            }
            // Reconciliar tiles gated con overlays activos: si el archivo
            // dice "Uranian on" pero no hay TileId::Uraniano en el order,
            // lo agregamos. Y al revés: tile dinámico sin overlay → fuera.
            reconcile(&mut s);
            s
        }
        Err(e) => {
            eprintln!("cosmos · ui-state: no se pudo parsear {path:?}: {e}");
            UiState::default()
        }
    }
}

/// Reescribe `s.panel_order` para mantener la invariante:
///   - todas las always-on están presentes
///   - cada TileId gated existe sii su overlay está en `s.overlays`
fn reconcile(s: &mut UiState) {
    let active_gated: Vec<TileId> = s
        .overlays
        .iter()
        .filter_map(|k| overlay_tile(*k))
        .collect();
    s.panel_order.retain(|tid| {
        is_always_on(*tid) || active_gated.contains(tid)
    });
    for tid in &active_gated {
        if !s.panel_order.contains(tid) {
            s.panel_order.push(*tid);
        }
    }
}

fn is_always_on(tid: TileId) -> bool {
    DEFAULT_ORDER.contains(&tid)
}

fn save_ui_state(s: &UiState) {
    let Some(path) = ui_state_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(json) = serde_json::to_vec_pretty(s) else {
        return;
    };
    if let Err(e) = std::fs::write(&path, json) {
        eprintln!("cosmos · ui-state: write fallido {path:?}: {e}");
    }
}

/// Devuelve el TileId dinámico que aporta un overlay, si aporta uno. Los
/// overlays que solo tienen efecto visual sobre el wheel (Lots, FixedStars,
/// Midpoints) no agregan tile propio — su toggle vive en el tile Módulos.
fn overlay_tile(kind: OverlayKind) -> Option<TileId> {
    match kind {
        OverlayKind::Uranian => Some(TileId::Uraniano),
        OverlayKind::Transit => Some(TileId::CrossTransit),
        OverlayKind::Progression => Some(TileId::CrossProgression),
        OverlayKind::SolarArc => Some(TileId::CrossSolarArc),
        OverlayKind::Lots => Some(TileId::Lotes),
        OverlayKind::FixedStars => Some(TileId::EstrellasFijas),
        OverlayKind::Midpoints => Some(TileId::PuntosMedios),
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
    /// Watcher del archivo `cosmos-chart.json`. Lo mantenemos vivo en el
    /// Model — se cae automáticamente cuando la app cierra.
    _chart_watcher: Option<notify::RecommendedWatcher>,
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
        TileId::Cualidades => tile_cualidades(&model.render, theme),
        TileId::AstroCarto => tile_astrocarto(&model.chart, &model.render, theme),
        TileId::Cartas => tile_cartas(theme),
        TileId::Corpus => tile_corpus(&model.render, &model.corpus, theme),
        TileId::Uraniano => tile_uraniano(&model.render.uranian_groups, theme),
        TileId::Lotes => tile_layer_glyphs(&model.render, LayerKind::Lots, "lots", theme),
        TileId::EstrellasFijas => {
            tile_layer_glyphs(&model.render, LayerKind::FixedStars, "fixed_stars", theme)
        }
        TileId::PuntosMedios => {
            tile_layer_glyphs(&model.render, LayerKind::Midpoints, "midpoints", theme)
        }
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

/// Fila de la tabla de aspectos — columnas: tipo (con color), par
/// de cuerpos, orbe, dirección (applying/separating). Usamos un Row
/// con cells de ancho fijo para que las columnas se alineen entre
/// filas.
#[allow(clippy::too_many_arguments)]
fn aspect_row(
    kind_code: &str,
    from: &str,
    to: &str,
    orb_dms: &str,
    dir: &str,
    kind_id: &str,
    theme: &Theme,
) -> View<Msg> {
    let size = 11.0_f32;
    let kind_color = aspecto_color(kind_id);
    let cell = |text: String, color: Color, w: f32| -> View<Msg> {
        View::new(Style {
            size: Size {
                width: length(w),
                height: length(size + 4.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(text, size, color, Alignment::Start)
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(size + 4.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![
        cell(kind_code.to_string(), kind_color, 28.0),
        cell(from.to_string(), theme.fg_text, 38.0),
        cell(to.to_string(), theme.fg_text, 38.0),
        cell(orb_dms.to_string(), theme.fg_muted, 56.0),
        cell(dir.to_string(), theme.fg_muted, 22.0),
    ])
}

// ----- Cartas (librería multi-archivo) -----

fn tile_cartas(theme: &Theme) -> View<Msg> {
    let pal_btn = ButtonPalette::from_theme(theme);
    let mut rows: Vec<View<Msg>> = Vec::new();

    // Botón "duplicar actual" arriba — captura el chart presente en disco.
    rows.push(button_styled(
        rimay_localize::t("cosmos-cartas-duplicar"),
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
            margin: Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(4.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Start),
            ..Default::default()
        },
        Alignment::Start,
        &pal_btn,
        Msg::DuplicarActual,
    ));

    let cards = list_cards();
    if cards.is_empty() {
        rows.push(line(
            rimay_localize::t("cosmos-cartas-vacio"),
            10.0,
            theme.fg_muted,
        ));
    } else {
        for name in cards {
            rows.push(button_styled(
                name.clone(),
                Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(20.0_f32),
                    },
                    flex_shrink: 0.0,
                    padding: Rect {
                        left: length(8.0_f32),
                        right: length(8.0_f32),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    margin: Rect {
                        left: length(0.0_f32),
                        right: length(0.0_f32),
                        top: length(0.0_f32),
                        bottom: length(2.0_f32),
                    },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::Start),
                    ..Default::default()
                },
                Alignment::Start,
                &pal_btn,
                Msg::CargarCarta(name),
            ));
        }
    }

    let path_hint = charts_dir()
        .map(|p| format!("dir: {}", p.display()))
        .unwrap_or_default();
    rows.push(line(path_hint, 9.0, theme.fg_muted));

    tile_container(rows, theme)
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

    let path_hint = chart_path()
        .map(|p| format!("edit: {}", p.display()))
        .unwrap_or_default();
    tile_container(
        vec![
            line(model.chart.label.clone(), 12.0, theme.fg_text),
            line(lugar, 10.0, theme.fg_muted),
            line(fecha, 10.0, theme.fg_muted),
            line(lat_long, 10.0, theme.fg_muted),
            line(angles, 10.0, theme.fg_text),
            line(path_hint, 9.0, theme.fg_muted),
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
            // "℞" (U+211E) no está en LiberationSans — usamos "(R)".
            let retro = if g.retrograde { " (R)" } else { "" };
            let dignity = g.dignity_marker.clone().unwrap_or_default();
            let line_str = format!("{body} {dms} {sign}{casa}{retro}{dignity}");
            line(line_str, 11.0, theme.fg_text)
        })
        .collect();
    tile_container(rows, theme)
}

// ----- Aspectos (filtrado por module_id) -----

/// Ranking de importancia del aspecto: 0 = mayor (con/opp/squ/tri/sex),
/// 1 = menor (quincunx/semi-sextile/semi-square/sesquiquadrate/quintile).
/// Sortear primero por esta clave, después por orb ascendente, da la
/// tabla con los aspectos más fuertes y cerrados arriba.
fn aspecto_importancia(kind: &str) -> u8 {
    match kind {
        "conjunction" | "opposition" | "square" | "trine" | "sextile" => 0,
        _ => 1,
    }
}

/// Color del aspecto desde la paleta agnóstica de `cosmos-render`,
/// traducido al `peniko::Color` que usa Llimphi. Mismas decisiones
/// cromáticas que el wheel — así el color del aspecto en la tabla
/// concuerda con el color de la línea en el wheel.
fn aspecto_color(kind: &str) -> Color {
    let pal = cosmos_render::Palette::dark();
    let c = pal.aspect(kind);
    let to_byte = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::from_rgba8(to_byte(c.r), to_byte(c.g), to_byte(c.b), to_byte(c.a))
}

fn tile_aspectos(render: &RenderModel, module_id: &str, theme: &Theme) -> View<Msg> {
    let mut asps: Vec<&AspectSummary> = render
        .aspect_summary
        .iter()
        .filter(|a| a.module_id == module_id)
        .collect();
    // Orden: primero los mayores; dentro de cada grupo, por orbe
    // ascendente (los más exactos arriba).
    asps.sort_by(|a, b| {
        aspecto_importancia(&a.kind)
            .cmp(&aspecto_importancia(&b.kind))
            .then_with(|| {
                a.orb_deg
                    .partial_cmp(&b.orb_deg)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    let rows: Vec<View<Msg>> = asps
        .into_iter()
        .take(20)
        .map(|a| {
            let from = simbolo_cuerpo(&a.from_body);
            let to = simbolo_cuerpo(&a.to_body);
            let kind = simbolo_aspecto(&a.kind);
            let dms = fmt_dms(a.orb_deg);
            // ◂ ▸ (U+25C2/B8) no están en LiberationSans — usamos
            // ◄ ► (U+25C4/BA) que sí, y son visualmente idénticos.
            let dir = match a.applying {
                Some(true) => " ◄",
                Some(false) => " ►",
                None => "",
            };
            // El kind va coloreado por la paleta de aspectos; el
            // resto en fg_text — así la columna de tipo se distingue
            // visualmente de los nombres de cuerpo.
            aspect_row(kind, from, to, &dms, dir, a.kind.as_str(), theme)
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

// ----- Cualidades (elementos + modalidades + polaridad) -----

fn tile_cualidades(render: &RenderModel, theme: &Theme) -> View<Msg> {
    let bodies: Vec<(&str, f32)> = render
        .layers
        .iter()
        .filter(|l| l.module_id == "natal" && matches!(l.kind, LayerKind::Bodies))
        .flat_map(|l| l.glyphs.iter())
        .map(|g| (g.symbol.as_str(), g.deg))
        .collect();

    // Elementos: sign_idx % 4 → 0=Fuego, 1=Tierra, 2=Aire, 3=Agua.
    let mut elementos: [Vec<&'static str>; 4] = Default::default();
    // Modalidades: sign_idx % 3 → 0=Cardinal, 1=Fijo, 2=Mutable.
    let mut modalidades: [Vec<&'static str>; 3] = Default::default();
    // Polaridad: sign_idx % 2 → 0=Yang (fuego/aire), 1=Yin (tierra/agua).
    let mut polaridad: [Vec<&'static str>; 2] = Default::default();

    for (name, deg) in &bodies {
        let sign_idx = ((deg.rem_euclid(360.0) / 30.0) as usize) % 12;
        let glyph = simbolo_cuerpo(name);
        elementos[sign_idx % 4].push(glyph);
        modalidades[sign_idx % 3].push(glyph);
        polaridad[sign_idx % 2].push(glyph);
    }

    let elem_labels = [
        rimay_localize::t("cosmos-elem-fuego"),
        rimay_localize::t("cosmos-elem-tierra"),
        rimay_localize::t("cosmos-elem-aire"),
        rimay_localize::t("cosmos-elem-agua"),
    ];
    let mod_labels = [
        rimay_localize::t("cosmos-mod-cardinal"),
        rimay_localize::t("cosmos-mod-fijo"),
        rimay_localize::t("cosmos-mod-mutable"),
    ];
    let pol_labels = [
        rimay_localize::t("cosmos-pol-yang"),
        rimay_localize::t("cosmos-pol-yin"),
    ];

    let mut rows: Vec<View<Msg>> = Vec::new();
    rows.push(seccion_label(rimay_localize::t("cosmos-elementos"), theme));
    for (i, label) in elem_labels.iter().enumerate() {
        rows.push(fila_cualidad(label, &elementos[i], theme));
    }
    rows.push(seccion_label(rimay_localize::t("cosmos-modalidades"), theme));
    for (i, label) in mod_labels.iter().enumerate() {
        rows.push(fila_cualidad(label, &modalidades[i], theme));
    }
    rows.push(seccion_label(rimay_localize::t("cosmos-polaridad"), theme));
    for (i, label) in pol_labels.iter().enumerate() {
        rows.push(fila_cualidad(label, &polaridad[i], theme));
    }
    tile_container(rows, theme)
}

fn seccion_label(text: String, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(14.0_f32),
        },
        flex_shrink: 0.0,
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(2.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(text, 10.0, theme.fg_muted, Alignment::Start)
}

fn fila_cualidad(label: &str, glyphs: &[&str], theme: &Theme) -> View<Msg> {
    let count = glyphs.len();
    let bar_len = count.min(10);
    // █ y ░ existen en LiberationSans/AdwaitaSans; ▰/▱ no, por eso
    // antes salían como cajitas .notdef.
    let bar: String = "█".repeat(bar_len) + &"░".repeat(10_usize.saturating_sub(bar_len));
    let glyph_str = glyphs.join(" ");
    let txt = format!("{label:>9}  {bar}  {count}  {glyph_str}");
    line(txt, 11.0, theme.fg_text)
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

// ----- Tile genérico: lista de glyphs de una layer -----

fn tile_layer_glyphs(
    render: &RenderModel,
    kind: LayerKind,
    module_id: &str,
    theme: &Theme,
) -> View<Msg> {
    let glyphs: Vec<&cosmos_render::Glyph> = render
        .layers
        .iter()
        .filter(|l| l.module_id == module_id && std::mem::discriminant(&l.kind) == std::mem::discriminant(&kind))
        .flat_map(|l| l.glyphs.iter())
        .collect();
    if glyphs.is_empty() {
        return tile_container(
            vec![line(
                rimay_localize::t("cosmos-empty"),
                11.0,
                theme.fg_muted,
            )],
            theme,
        );
    }
    let rows: Vec<View<Msg>> = glyphs
        .into_iter()
        .take(20)
        .map(|g| {
            // Para lots viene "lot:Fo" — recortamos el prefijo y usamos annotation.
            let label = if g.symbol.starts_with("lot:") {
                g.annotation.clone().unwrap_or_else(|| g.symbol.clone())
            } else if g.symbol.starts_with("✦") {
                g.annotation.clone().unwrap_or_else(|| g.symbol.clone())
            } else {
                simbolo_cuerpo(&g.symbol).to_string()
            };
            let casa = g.house.map(|h| format!(" h{h}")).unwrap_or_default();
            let dms = fmt_dms(g.deg.rem_euclid(30.0) as f64);
            let sign = signo_de_longitud(g.deg);
            line(
                format!("{label}  {dms} {sign}{casa}"),
                11.0,
                theme.fg_text,
            )
        })
        .collect();
    tile_container(rows, theme)
}

fn sorted_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.into(), b.into())
    } else {
        (b.into(), a.into())
    }
}

// ----- AstroCarto (MC/IC + Asc/Desc líneas sobre mapa equirectangular) -----
//
// MVP sin fondo de continentes — solo grilla lat/long y las líneas de los
// cuerpos clásicos. La aproximación supone latitud eclíptica β=0 para
// todos los cuerpos (válido para AstroCarto a este zoom; Luna y Plutón
// se separan unos grados pero la silueta de líneas es la misma). La
// obliquidad usa ε₂₀₀₀ = 23.4393° fijo — el error a 100 años es <0.01°.

const ASTROCARTO_OBLIQUITY: f64 = 23.4393;
const ASTROCARTO_W: f32 = 320.0;
const ASTROCARTO_H: f32 = 160.0;

fn julian_day_utc(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: f64) -> f64 {
    let (y, m) = if month <= 2 {
        (year - 1, (month + 12) as i32)
    } else {
        (year, month as i32)
    };
    let a = (y as f64 / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    let jd0 = (365.25 * (y as f64 + 4716.0)).floor()
        + (30.6001 * (m as f64 + 1.0)).floor()
        + day as f64
        + b
        - 1524.5;
    let frac = (hour as f64 + minute as f64 / 60.0 + second / 3600.0) / 24.0;
    jd0 + frac
}

/// GMST en grados [0, 360) — Meeus 12.4.
fn gmst_deg(jd_ut: f64) -> f64 {
    let t = (jd_ut - 2451545.0) / 36525.0;
    let g = 280.46061837
        + 360.98564736629 * (jd_ut - 2451545.0)
        + 0.000387933 * t * t
        - t * t * t / 38710000.0;
    g.rem_euclid(360.0)
}

/// Conversión ecliptica → ecuatorial con β=0 fijo. Retorna (RA°, Dec°).
fn ecliptic_to_equatorial(lon_deg: f64) -> (f64, f64) {
    let l = lon_deg.to_radians();
    let e = ASTROCARTO_OBLIQUITY.to_radians();
    let ra = (l.sin() * e.cos()).atan2(l.cos()).to_degrees().rem_euclid(360.0);
    let dec = (e.sin() * l.sin()).asin().to_degrees();
    (ra, dec)
}

/// Color por cuerpo en el AstroCarto. Hue distintivo para que las líneas
/// se diferencien aun cuando se cruzan.
fn color_de_cuerpo(name: &str) -> Color {
    match name {
        "sun" => Color::from_rgba8(255, 200, 60, 255),
        "moon" => Color::from_rgba8(200, 210, 220, 255),
        "mercury" => Color::from_rgba8(180, 180, 180, 255),
        "venus" => Color::from_rgba8(120, 220, 130, 255),
        "mars" => Color::from_rgba8(230, 90, 90, 255),
        "jupiter" => Color::from_rgba8(240, 170, 80, 255),
        "saturn" => Color::from_rgba8(180, 150, 90, 255),
        "uranus" => Color::from_rgba8(100, 220, 220, 255),
        "neptune" => Color::from_rgba8(100, 130, 230, 255),
        "pluto" => Color::from_rgba8(170, 90, 130, 255),
        _ => Color::from_rgba8(140, 140, 140, 255),
    }
}

/// Proyección equirectangular a coordenadas locales del canvas.
fn project_lon_lat(lon_deg: f64, lat_deg: f64) -> (f32, f32) {
    let x = ((lon_deg + 180.0) / 360.0) as f32 * ASTROCARTO_W;
    let y = ((90.0 - lat_deg) / 180.0) as f32 * ASTROCARTO_H;
    (x.clamp(0.0, ASTROCARTO_W), y.clamp(0.0, ASTROCARTO_H))
}

fn tile_astrocarto(chart: &Chart, render: &RenderModel, theme: &Theme) -> View<Msg> {
    let bd = &chart.birth_data;
    // Local → UTC.
    let total_minutes_local = bd.hour as i64 * 60 + bd.minute as i64;
    let total_minutes_utc = total_minutes_local - bd.tz_offset_minutes as i64;
    let h_utc = total_minutes_utc as f64 / 60.0;
    let jd = julian_day_utc(bd.year, bd.month, bd.day, 0, 0, bd.second) + h_utc / 24.0;
    let gmst = gmst_deg(jd);
    // Cosas owned para meter dentro de la closure 'static.
    let natal_lat = bd.latitude_deg;
    let natal_lon = bd.longitude_deg;

    // Cuerpos natales con su longitud eclíptica.
    let bodies: Vec<(String, f64)> = render
        .layers
        .iter()
        .filter(|l| l.module_id == "natal" && matches!(l.kind, LayerKind::Bodies))
        .flat_map(|l| l.glyphs.iter())
        .map(|g| (g.symbol.clone(), g.deg as f64))
        .collect();

    let bg = theme.bg_panel_alt;
    let grid = theme.fg_muted;
    let canvas = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(ASTROCARTO_H + 4.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .paint_with(move |scene, _ts, rect: llimphi_ui::PaintRect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Stroke};
        use llimphi_ui::llimphi_raster::peniko::Color as PColor;
        // Aspect-fit centrado del canvas lógico ASTROCARTO_WxH al rect.
        let scale_x = rect.w as f64 / ASTROCARTO_W as f64;
        let scale_y = rect.h as f64 / ASTROCARTO_H as f64;
        let scale = scale_x.min(scale_y);
        let disp_w = ASTROCARTO_W as f64 * scale;
        let disp_h = ASTROCARTO_H as f64 * scale;
        let off_x = rect.x as f64 + (rect.w as f64 - disp_w) * 0.5;
        let off_y = rect.y as f64 + (rect.h as f64 - disp_h) * 0.5;
        let xform = Affine::translate((off_x, off_y)) * Affine::scale(scale);

        // Grilla: ecuador, ±30°, ±60°.
        let grid_color = PColor::from_rgba8(
            (grid.components[0] * 255.0) as u8,
            (grid.components[1] * 255.0) as u8,
            (grid.components[2] * 255.0) as u8,
            80,
        );
        for lat in [-60.0_f64, -30.0, 0.0, 30.0, 60.0] {
            let (_, y) = project_lon_lat(0.0, lat);
            let mut p = BezPath::new();
            p.move_to((0.0, y as f64));
            p.line_to((ASTROCARTO_W as f64, y as f64));
            scene.stroke(&Stroke::new(0.5), xform, grid_color, None, &p);
        }
        // Líneas verticales cada 60° de longitud.
        for lon in [-120.0_f64, -60.0, 0.0, 60.0, 120.0] {
            let (x, _) = project_lon_lat(lon, 0.0);
            let mut p = BezPath::new();
            p.move_to((x as f64, 0.0));
            p.line_to((x as f64, ASTROCARTO_H as f64));
            scene.stroke(&Stroke::new(0.5), xform, grid_color, None, &p);
        }

        for (name, ecl_lon) in &bodies {
            let (ra, dec) = ecliptic_to_equatorial(*ecl_lon);
            let mc_lon = wrap_lon(ra - gmst);
            let ic_lon = wrap_lon(mc_lon + 180.0);
            let body_color = color_de_cuerpo(name);

            // MC: línea vertical a lo largo del canvas.
            let (x_mc, _) = project_lon_lat(mc_lon, 0.0);
            let mut p = BezPath::new();
            p.move_to((x_mc as f64, 0.0));
            p.line_to((x_mc as f64, ASTROCARTO_H as f64));
            scene.stroke(&Stroke::new(1.4), xform, body_color, None, &p);

            // IC: línea vertical punteada para distinguir.
            let (x_ic, _) = project_lon_lat(ic_lon, 0.0);
            let mut p = BezPath::new();
            p.move_to((x_ic as f64, 0.0));
            p.line_to((x_ic as f64, ASTROCARTO_H as f64));
            scene.stroke(
                &Stroke::new(1.0).with_dashes(0.0, [4.0, 4.0]),
                xform,
                body_color,
                None,
                &p,
            );

            // Asc/Desc: curvas paramétricas en latitud.
            if dec.abs() < 89.9 {
                let mut rise = BezPath::new();
                let mut set = BezPath::new();
                let mut rise_started = false;
                let mut set_started = false;
                let dec_r = dec.to_radians();
                let mut phi_deg = -85.0_f64;
                while phi_deg <= 85.0 {
                    let phi_r = phi_deg.to_radians();
                    let cos_h = -phi_r.tan() * dec_r.tan();
                    if cos_h.abs() <= 1.0 {
                        let h_deg = cos_h.acos().to_degrees();
                        let lon_r = wrap_lon(ra - h_deg - gmst);
                        let lon_s = wrap_lon(ra + h_deg - gmst);
                        let (xr, yr) = project_lon_lat(lon_r, phi_deg);
                        let (xs, ys) = project_lon_lat(lon_s, phi_deg);
                        if rise_started {
                            rise.line_to((xr as f64, yr as f64));
                        } else {
                            rise.move_to((xr as f64, yr as f64));
                            rise_started = true;
                        }
                        if set_started {
                            set.line_to((xs as f64, ys as f64));
                        } else {
                            set.move_to((xs as f64, ys as f64));
                            set_started = true;
                        }
                    } else if rise_started || set_started {
                        // Cruzamos región circumpolar — corta la línea.
                        rise_started = false;
                        set_started = false;
                    }
                    phi_deg += 3.0;
                }
                if rise_started {
                    scene.stroke(&Stroke::new(0.8), xform, body_color, None, &rise);
                }
                if set_started {
                    scene.stroke(&Stroke::new(0.8), xform, body_color, None, &set);
                }
            }
        }

        // Marca del lugar de nacimiento.
        let (px, py) = project_lon_lat(natal_lon, natal_lat);
        let mark = llimphi_ui::llimphi_raster::kurbo::Circle::new(
            (px as f64, py as f64),
            3.0,
        );
        scene.fill(
            llimphi_ui::llimphi_raster::peniko::Fill::NonZero,
            xform,
            PColor::from_rgba8(255, 255, 255, 230),
            None,
            &mark,
        );
    });

    tile_container(
        vec![
            canvas,
            line(
                rimay_localize::t("cosmos-astrocarto-leyenda"),
                9.0,
                theme.fg_muted,
            ),
        ],
        theme,
    )
}

fn wrap_lon(lon: f64) -> f64 {
    let l = lon.rem_euclid(360.0);
    if l > 180.0 {
        l - 360.0
    } else {
        l
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

/// Códigos alfabéticos para mostrar cuerpos en los tiles del sidebar.
/// **Por qué letras y no unicode** (☉☽☿…): las fuentes default del
/// sistema (LiberationSans, AdwaitaSans) no traen `U+2609..U+265F`,
/// así que cualquier glyph astrológico cae como `.notdef`. En el
/// wheel ya los dibujamos como path (`cosmos_render::glyphs`) — acá
/// son texto plano dentro de filas, así que usamos códigos cortos.
fn simbolo_cuerpo(s: &str) -> &'static str {
    match s {
        "sun" => "Sol",
        "moon" => "Lun",
        "mercury" => "Mer",
        "venus" => "Ven",
        "mars" => "Mar",
        "jupiter" => "Jup",
        "saturn" => "Sat",
        "uranus" => "Ura",
        "neptune" => "Nep",
        "pluto" => "Plu",
        "earth" => "Tie",
        "north_node" | "ascending_node" => "NoN",
        "south_node" | "descending_node" => "NoS",
        "lilith" => "Lil",
        "chiron" => "Qui",
        "mean_node" => "NoN",
        "asc" => "Asc",
        "desc" => "Dsc",
        "mc" => "MC",
        "ic" => "IC",
        _ => "·",
    }
}

/// Códigos alfabéticos para tipos de aspecto. Mismo motivo que
/// [`simbolo_cuerpo`] — los unicode ☌☍△□⚹ no rendean.
fn simbolo_aspecto(s: &str) -> &'static str {
    match s {
        "conjunction" => "con",
        "opposition" => "opp",
        "trine" => "tri",
        "square" => "cua",
        "sextile" => "sex",
        "quincunx" => "qui",
        "semi_sextile" => "ssx",
        "semi_square" => "scu",
        "sesquiquadrate" => "scq",
        _ => "·",
    }
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Cosmos>();
}
