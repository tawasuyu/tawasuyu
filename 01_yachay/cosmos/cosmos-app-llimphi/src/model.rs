//! Modelo de la app, mensajes del bucle Elm y la taxonomía de tiles/overlays.

use cosmos_engine::{Corpus, PipelineRequest};
use cosmos_model::Chart;
use cosmos_render::RenderModel;
use llimphi_theme::Theme;
use serde::{Deserialize, Serialize};

pub(crate) const WHEEL_SIZE: f32 = 720.0;
pub(crate) const SIDEBAR_WIDTH: f32 = 340.0;
pub(crate) const HARMONICS: &[u32] = &[1, 4, 5, 7, 9];

#[derive(Clone)]
pub(crate) enum Msg {
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
    /// Expandir/colapsar el nodo raíz "Biblioteca" del árbol de cartas.
    ToggleBiblioteca,
    /// Seleccionar (o deseleccionar con `None`) un cuerpo natal. Se
    /// dispara desde el `on_click_at` del canvas; al estar
    /// seleccionado, el wheel atenúa los cuerpos y aspectos no
    /// relacionados con el elegido.
    SelectBody(Option<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OverlayKind {
    Transit,
    Progression,
    SolarArc,
    Uranian,
    Lots,
    FixedStars,
    Midpoints,
}

impl OverlayKind {
    pub(crate) fn all() -> &'static [OverlayKind] {
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

    pub(crate) fn label(self) -> &'static str {
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

    pub(crate) fn to_request(self, target_age: f64) -> PipelineRequest {
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
pub(crate) enum TileId {
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
    pub(crate) fn label_key(self) -> &'static str {
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

pub(crate) const DEFAULT_ORDER: &[TileId] = &[
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

/// Devuelve el TileId dinámico que aporta un overlay, si aporta uno. Los
/// overlays que solo tienen efecto visual sobre el wheel (Lots, FixedStars,
/// Midpoints) no agregan tile propio — su toggle vive en el tile Módulos.
pub(crate) fn overlay_tile(kind: OverlayKind) -> Option<TileId> {
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

pub(crate) struct Model {
    pub(crate) chart: Chart,
    pub(crate) overlays: Vec<OverlayKind>,
    pub(crate) harmonic: u32,
    pub(crate) render: RenderModel,
    pub(crate) theme: Theme,
    pub(crate) error: Option<String>,
    /// Orden actual de tiles visibles. Se reordena por drag-to-swap; se
    /// extiende/recorta cuando los overlays con tile propio se prenden/apagan.
    pub(crate) panel_order: Vec<TileId>,
    pub(crate) corpus: Corpus,
    /// Si el nodo raíz "Biblioteca" del árbol de cartas está expandido.
    pub(crate) lib_expanded: bool,
    /// Nombre de la carta de biblioteca actualmente cargada (`None` si la
    /// carta presente viene del `cosmos-chart.json` editado a mano y no de
    /// un archivo de la biblioteca). Resalta la fila en el árbol.
    pub(crate) selected_card: Option<String>,
    /// Cuerpo natal seleccionado (`None` = sin selección). Cuando hay
    /// selección, el wheel atenúa los cuerpos y aspectos que no lo
    /// involucran; click en vacío deselecciona.
    pub(crate) selected_body: Option<String>,
    pub(crate) _wawa_watcher: Option<wawa_config::ConfigWatcher>,
    /// Watcher del archivo `cosmos-chart.json`. Lo mantenemos vivo en el
    /// Model — se cae automáticamente cuando la app cierra.
    pub(crate) _chart_watcher: Option<notify::RecommendedWatcher>,
}
