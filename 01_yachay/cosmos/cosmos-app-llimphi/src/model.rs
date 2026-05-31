//! Modelo del shell, mensajes del bucle Elm y las taxonomías de
//! vistas/capas/menús.
//!
//! El shell es un IDE astronómico/astrológico: barra de menú principal
//! arriba, árbol de navegación a la izquierda (cartas + catálogo de
//! gráficas), pestañas en el área central (una por gráfica abierta) y
//! barra de estado abajo. Menús contextuales (click derecho) sobre la
//! rueda. Todo lo configurable vive en la vista `Configuración` y en el
//! menú `Capas`/`Armónico`.

use cosmos_engine::{Corpus, PipelineRequest};
use cosmos_model::Chart;
use cosmos_render::RenderModel;
use llimphi_theme::Theme;
use serde::{Deserialize, Serialize};

use crate::astroview::AstroState;

pub(crate) const WHEEL_SIZE: f32 = 720.0;
pub(crate) const NAV_WIDTH: f32 = 232.0;
pub(crate) const MENU_BAR_H: f32 = 30.0;
pub(crate) const TAB_BAR_H: f32 = 30.0;
pub(crate) const STATUS_H: f32 = 22.0;
pub(crate) const HARMONICS: &[u32] = &[1, 4, 5, 7, 9];

/// Origen X de la primera entrada de menú (después del pill "cosmos").
pub(crate) const MENU_X0: f32 = 84.0;
/// Ancho fijo de cada botón de la barra de menú — fija el anclaje del
/// dropdown sin medir el texto.
pub(crate) const MENU_BTN_W: f32 = 84.0;

/// Viewport asumido para clamping de overlays. La ventana puede
/// redimensionarse; usamos el tamaño inicial como aproximación (el
/// trait `App` no expone resize). Suficiente para que el dropdown no se
/// salga por abajo/derecha en el tamaño por defecto.
pub(crate) const VIEWPORT: (f32, f32) = (1200.0, 860.0);

// =====================================================================
// Vistas (gráficas) — cada una se abre como pestaña
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ViewKind {
    // Astrología
    Rueda,
    Cuerpos,
    Aspectos,
    BoxGraph,
    Cualidades,
    Uraniano,
    Lotes,
    EstrellasFijas,
    PuntosMedios,
    Corpus,
    AstroCarto,
    // Astronomía
    Cielo,
    OrtoOcaso,
    Sundial,
    Mareas,
    Eclipses,
    Efemerides,
    // Sistema
    Configuracion,
}

impl ViewKind {
    pub(crate) fn title(self) -> &'static str {
        match self {
            ViewKind::Rueda => "Rueda natal",
            ViewKind::Cuerpos => "Cuerpos",
            ViewKind::Aspectos => "Aspectos",
            ViewKind::BoxGraph => "Aspectario",
            ViewKind::Cualidades => "Cualidades",
            ViewKind::Uraniano => "Uraniano",
            ViewKind::Lotes => "Lotes",
            ViewKind::EstrellasFijas => "Estrellas fijas",
            ViewKind::PuntosMedios => "Puntos medios",
            ViewKind::Corpus => "Interpretación",
            ViewKind::AstroCarto => "AstroCartografía",
            ViewKind::Cielo => "Cielo (alt/az)",
            ViewKind::OrtoOcaso => "Orto y ocaso",
            ViewKind::Sundial => "Reloj de sol",
            ViewKind::Mareas => "Mareas",
            ViewKind::Eclipses => "Eclipses",
            ViewKind::Efemerides => "Efemérides",
            ViewKind::Configuracion => "Configuración",
        }
    }

    /// Gráficas astrológicas, en orden de aparición en el árbol.
    pub(crate) fn astrologia() -> &'static [ViewKind] {
        &[
            ViewKind::Rueda,
            ViewKind::Cuerpos,
            ViewKind::Aspectos,
            ViewKind::BoxGraph,
            ViewKind::Cualidades,
            ViewKind::Uraniano,
            ViewKind::Lotes,
            ViewKind::EstrellasFijas,
            ViewKind::PuntosMedios,
            ViewKind::Corpus,
            ViewKind::AstroCarto,
        ]
    }

    /// Gráficas astronómicas (no astrológicas) sobre el mismo motor.
    pub(crate) fn astronomia() -> &'static [ViewKind] {
        &[
            ViewKind::Cielo,
            ViewKind::OrtoOcaso,
            ViewKind::Sundial,
            ViewKind::Mareas,
            ViewKind::Eclipses,
            ViewKind::Efemerides,
        ]
    }
}

// =====================================================================
// Capas (overlays) que se superponen a la carta natal
// =====================================================================

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

    /// Nombre legible en español para el menú `Capas` y la vista de
    /// configuración. (Los keys fluent siguen en `cosmos-overlay-*` pero
    /// el chrome nuevo usa literales para no acoplar a la i18n.)
    pub(crate) fn nombre(self) -> &'static str {
        match self {
            OverlayKind::Transit => "Tránsitos",
            OverlayKind::Progression => "Progresiones",
            OverlayKind::SolarArc => "Arco solar",
            OverlayKind::Uranian => "Uraniano",
            OverlayKind::Lots => "Lotes",
            OverlayKind::FixedStars => "Estrellas fijas",
            OverlayKind::Midpoints => "Puntos medios",
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

// =====================================================================
// Menú principal y opciones configurables
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuKind {
    Archivo,
    Vista,
    Capas,
    Armonico,
    Ayuda,
}

impl MenuKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            MenuKind::Archivo => "Archivo",
            MenuKind::Vista => "Vista",
            MenuKind::Capas => "Capas",
            MenuKind::Armonico => "Armónico",
            MenuKind::Ayuda => "Ayuda",
        }
    }

    pub(crate) fn order() -> &'static [MenuKind] {
        &[
            MenuKind::Archivo,
            MenuKind::Vista,
            MenuKind::Capas,
            MenuKind::Armonico,
            MenuKind::Ayuda,
        ]
    }

    /// X de anclaje del dropdown bajo el botón de la barra.
    pub(crate) fn anchor_x(self) -> f32 {
        let idx = Self::order().iter().position(|m| *m == self).unwrap_or(0);
        MENU_X0 + idx as f32 * MENU_BTN_W
    }
}

/// Grupos colapsables del árbol de navegación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavGroup {
    Cartas,
    Astrologia,
    Astronomia,
    Sistema,
}

/// Opción booleana del wheel — togglada desde el menú contextual y la
/// vista de configuración.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WheelOpt {
    MinorAspects,
    CoordLabels,
    Dial3d,
    AscCross,
}

/// Configuración persistente del visor: tema, opciones del wheel,
/// instante de cómputo astronómico.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CosmosConfig {
    pub(crate) theme_dark: bool,
    pub(crate) minor_aspects: bool,
    pub(crate) coord_labels: bool,
    pub(crate) dial_3d: bool,
    pub(crate) asc_cross: bool,
    pub(crate) rot_offset_deg: f32,
    /// `true` = las gráficas astronómicas usan el instante actual;
    /// `false` = usan el instante de la carta cargada.
    pub(crate) use_now: bool,
}

impl Default for CosmosConfig {
    fn default() -> Self {
        Self {
            theme_dark: true,
            minor_aspects: false,
            coord_labels: true,
            dial_3d: true,
            asc_cross: true,
            rot_offset_deg: 0.0,
            use_now: false,
        }
    }
}

// =====================================================================
// Mensajes del bucle Elm
// =====================================================================

#[derive(Clone)]
pub(crate) enum Msg {
    WawaConfigChanged(Box<wawa_config::WawaConfig>),
    // navegación
    SelectView(ViewKind),
    ActivateTab(usize),
    CloseTab(usize),
    ToggleNavGroup(NavGroup),
    CargarCarta(String),
    /// `cosmos-chart.json` cambió en disco — recargar.
    ChartFileChanged,
    SelectBody(Option<String>),
    // capas / armónico / configuración
    ToggleOverlay(OverlayKind),
    SetHarmonic(u32),
    SetThemeDark(bool),
    ToggleWheelOpt(WheelOpt),
    SetRotOffset(f32),
    SetUseNow(bool),
    // menú principal
    OpenMenu(MenuKind),
    MenuPick(MenuKind, usize),
    CloseMenu,
    // menú contextual sobre la rueda
    OpenCanvasCtx(f32, f32),
    CtxPick(usize),
    CloseCtx,
}

// =====================================================================
// Modelo
// =====================================================================

pub(crate) struct Model {
    pub(crate) chart: Chart,
    pub(crate) overlays: Vec<OverlayKind>,
    pub(crate) harmonic: u32,
    pub(crate) render: RenderModel,
    /// Lecturas astronómicas cacheadas (alt/az, sundial, mareas,
    /// orto/ocaso, eclipses). Recalculadas sólo al cambiar carta o el
    /// instante — nunca por frame.
    pub(crate) astro: AstroState,
    pub(crate) corpus: Corpus,
    pub(crate) cfg: CosmosConfig,
    pub(crate) theme: Theme,
    pub(crate) error: Option<String>,
    /// Nota efímera en la barra de estado (confirmaciones, "acerca de").
    pub(crate) status_note: Option<String>,
    // navegación
    pub(crate) tabs: Vec<ViewKind>,
    pub(crate) active_tab: usize,
    pub(crate) selected_card: Option<String>,
    pub(crate) selected_body: Option<String>,
    pub(crate) exp_cartas: bool,
    pub(crate) exp_astrologia: bool,
    pub(crate) exp_astronomia: bool,
    pub(crate) exp_sistema: bool,
    // chrome
    pub(crate) menu_open: Option<MenuKind>,
    pub(crate) ctx_open: Option<(f32, f32)>,
    // watchers
    pub(crate) _wawa_watcher: Option<wawa_config::ConfigWatcher>,
    pub(crate) _chart_watcher: Option<notify::RecommendedWatcher>,
}

impl Model {
    /// Vista activa (la de la pestaña seleccionada). Garantiza un valor
    /// aunque `tabs` esté momentáneamente vacío.
    pub(crate) fn active_view(&self) -> ViewKind {
        self.tabs
            .get(self.active_tab)
            .copied()
            .unwrap_or(ViewKind::Rueda)
    }

    pub(crate) fn toggle_group(&mut self, g: NavGroup) {
        match g {
            NavGroup::Cartas => self.exp_cartas = !self.exp_cartas,
            NavGroup::Astrologia => self.exp_astrologia = !self.exp_astrologia,
            NavGroup::Astronomia => self.exp_astronomia = !self.exp_astronomia,
            NavGroup::Sistema => self.exp_sistema = !self.exp_sistema,
        }
    }
}
