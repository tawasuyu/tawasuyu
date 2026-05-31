//! Modelo del shell, mensajes del bucle Elm y las taxonomías de
//! vistas/capas/menús.
//!
//! El shell es un IDE astronómico/astrológico: barra de menú principal
//! arriba, árbol de navegación a la izquierda (cartas + catálogo de
//! gráficas), pestañas en el área central (una por gráfica abierta) y
//! barra de estado abajo. Menús contextuales (click derecho) sobre la
//! rueda. Todo lo configurable vive en la vista `Configuración` y en el
//! menú `Capas`/`Armónico`.

use std::collections::HashSet;

use cosmos_engine::{Corpus, PipelineRequest};
use cosmos_model::Chart;
use cosmos_render::RenderModel;
use cosmos_store::Store;
use llimphi_theme::Theme;
use llimphi_widget_text_input::TextInputState;
use serde::{Deserialize, Serialize};

use crate::astroview::AstroState;
use crate::library::NavNode;

pub(crate) const WHEEL_SIZE: f32 = 720.0;
pub(crate) const NAV_WIDTH: f32 = 232.0;
pub(crate) const TOOLS_WIDTH: f32 = 360.0;
/// Rail de categorías del panel derecho (tabs verticales estilo Photoshop).
pub(crate) const TOOLS_RAIL_W: f32 = 40.0;
pub(crate) const MENU_BAR_H: f32 = 30.0;
pub(crate) const TAB_BAR_H: f32 = 30.0;
pub(crate) const STATUS_H: f32 = 22.0;
pub(crate) const HARMONICS: &[u32] = &[1, 4, 5, 7, 9];
/// Límites de arrastre de los paneles laterales guardables.
pub(crate) const NAV_MIN: f32 = 160.0;
pub(crate) const NAV_MAX: f32 = 460.0;
pub(crate) const TOOLS_MIN: f32 = 240.0;
pub(crate) const TOOLS_MAX: f32 = 620.0;

// =====================================================================
// Tipo de gráfica del centro (switcheable)
// =====================================================================

/// Qué dibuja el panel central. El usuario alterna con un segmented en la
/// cabecera del centro. La rueda estándar es el default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ChartView {
    /// Rueda natal 2D clásica (zodíaco + casas + aspectos).
    #[default]
    Estandar,
    /// Mapa equirectangular (AstroCartografía).
    Carto,
    /// Esfera celeste 3D (wireframe). Pendiente de cableo del renderer.
    Esfera3d,
    /// Cielo como lo ve el observador (alt/az). Pendiente de renderer
    /// gráfico — hoy hay tabla en el panel de herramientas.
    Cielo,
}

impl ChartView {
    pub(crate) fn title(self) -> &'static str {
        match self {
            ChartView::Estandar => "Estándar",
            ChartView::Carto => "Carto",
            ChartView::Esfera3d => "3D",
            ChartView::Cielo => "Cielo",
        }
    }

    pub(crate) fn all() -> &'static [ChartView] {
        &[
            ChartView::Estandar,
            ChartView::Carto,
            ChartView::Esfera3d,
            ChartView::Cielo,
        ]
    }
}

// =====================================================================
// Categorías del panel de herramientas (derecha)
// =====================================================================

/// Cada categoría es un contenedor de paneles que se intercambia con las
/// tabs verticales. `Principal` es la más usada (aspectos + cuerpos) y el
/// default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ToolCat {
    /// Lo más usado: aspectos (geocéntrico + topocéntrico) y cuerpos.
    #[default]
    Principal,
    /// Análisis astrológico avanzado (cualidades, uraniano, lotes…).
    Analisis,
    /// Lecturas astronómicas (cielo, orto/ocaso, mareas, eclipses…).
    Astronomia,
    /// Configuración del visor.
    Sistema,
}

impl ToolCat {
    pub(crate) fn title(self) -> &'static str {
        match self {
            ToolCat::Principal => "Principal",
            ToolCat::Analisis => "Análisis",
            ToolCat::Astronomia => "Astronomía",
            ToolCat::Sistema => "Sistema",
        }
    }

    /// Glifo corto para el rail vertical (estilo Photoshop).
    pub(crate) fn glyph(self) -> &'static str {
        match self {
            ToolCat::Principal => "△",
            ToolCat::Analisis => "✦",
            ToolCat::Astronomia => "☾",
            ToolCat::Sistema => "⚙",
        }
    }

    pub(crate) fn all() -> &'static [ToolCat] {
        &[
            ToolCat::Principal,
            ToolCat::Analisis,
            ToolCat::Astronomia,
            ToolCat::Sistema,
        ]
    }

    /// Paneles que viven en esta categoría, en orden de aparición.
    pub(crate) fn panels(self) -> &'static [ToolPanel] {
        match self {
            ToolCat::Principal => &[
                ToolPanel::Carta,
                ToolPanel::Aspectos,
                ToolPanel::AspectosTopo,
                ToolPanel::Cuerpos,
            ],
            ToolCat::Analisis => &[
                ToolPanel::Cualidades,
                ToolPanel::Uraniano,
                ToolPanel::BoxGraph,
                ToolPanel::Lotes,
                ToolPanel::EstrellasFijas,
                ToolPanel::PuntosMedios,
                ToolPanel::Corpus,
            ],
            ToolCat::Astronomia => &[
                ToolPanel::Cielo,
                ToolPanel::OrtoOcaso,
                ToolPanel::Sundial,
                ToolPanel::Mareas,
                ToolPanel::Eclipses,
                ToolPanel::Efemerides,
            ],
            ToolCat::Sistema => &[ToolPanel::Configuracion],
        }
    }
}

// =====================================================================
// Paneles de herramientas (colapsables) del panel derecho
// =====================================================================

/// Cada panel es una sección colapsable (acordeón) dentro de una
/// categoría. `Aspectos` y `AspectosTopo` arrancan expandidos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ToolPanel {
    Carta,
    Aspectos,
    AspectosTopo,
    Cuerpos,
    Cualidades,
    Uraniano,
    BoxGraph,
    Lotes,
    EstrellasFijas,
    PuntosMedios,
    Corpus,
    Cielo,
    OrtoOcaso,
    Sundial,
    Mareas,
    Eclipses,
    Efemerides,
    Configuracion,
}

impl ToolPanel {
    pub(crate) fn title(self) -> &'static str {
        match self {
            ToolPanel::Carta => "Datos de la carta",
            ToolPanel::Aspectos => "Aspectos",
            ToolPanel::AspectosTopo => "Aspectos · topocéntrico",
            ToolPanel::Cuerpos => "Cuerpos",
            ToolPanel::Cualidades => "Cualidades",
            ToolPanel::Uraniano => "Uraniano",
            ToolPanel::BoxGraph => "Aspectario",
            ToolPanel::Lotes => "Lotes",
            ToolPanel::EstrellasFijas => "Estrellas fijas",
            ToolPanel::PuntosMedios => "Puntos medios",
            ToolPanel::Corpus => "Interpretación",
            ToolPanel::Cielo => "Cielo (alt/az)",
            ToolPanel::OrtoOcaso => "Orto y ocaso",
            ToolPanel::Sundial => "Reloj de sol",
            ToolPanel::Mareas => "Mareas",
            ToolPanel::Eclipses => "Eclipses",
            ToolPanel::Efemerides => "Efemérides",
            ToolPanel::Configuracion => "Configuración",
        }
    }

    /// Paneles abiertos por defecto en una instalación nueva.
    pub(crate) fn defaults_expanded() -> Vec<ToolPanel> {
        vec![ToolPanel::Aspectos, ToolPanel::AspectosTopo]
    }
}

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
// Cartas abiertas (tabs del centro) — multi-carta
// =====================================================================

/// Una carta abierta como pestaña del centro. Guarda la carta completa
/// para poder alternar sin volver al store (y soporta cartas «scratch»
/// sin id). `render`/`astro` se recomputan al activar la pestaña.
#[derive(Debug, Clone)]
pub(crate) struct OpenTab {
    /// Id de la carta en el store (`None` = scratch / ejemplo no guardado).
    pub(crate) id: Option<String>,
    pub(crate) chart: Chart,
    /// Render cacheado de esta carta — permite pintar varias en mosaico
    /// sin recomputar por frame. Se recomputa al cambiar capas/armónico.
    pub(crate) render: RenderModel,
}

impl OpenTab {
    pub(crate) fn label(&self) -> &str {
        &self.chart.label
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
    /// Capa ascensional/topocéntrica: planetas en coordenadas del lugar.
    /// Activa por default — habilita la tabla de aspectos topocéntricos.
    Topocentric,
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
            OverlayKind::Topocentric,
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
            OverlayKind::Topocentric => "Topocéntrico",
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
            OverlayKind::Topocentric => PipelineRequest::Topocentric,
        }
    }
}

// =====================================================================
// Menú principal y opciones configurables
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuKind {
    Archivo,
    Editar,
    Vista,
    Capas,
    Armonico,
    Ayuda,
}

impl MenuKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            MenuKind::Archivo => "Archivo",
            MenuKind::Editar => "Editar",
            MenuKind::Vista => "Vista",
            MenuKind::Capas => "Capas",
            MenuKind::Armonico => "Armónico",
            MenuKind::Ayuda => "Ayuda",
        }
    }

    pub(crate) fn order() -> &'static [MenuKind] {
        &[
            MenuKind::Archivo,
            MenuKind::Editar,
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
    // multi-carta (tabs del centro)
    ActivateChartTab(usize),
    CloseChartTab(usize),
    /// Alterna entre vista de pestañas y mosaico (cartas lado a lado).
    ToggleTileMode,
    /// Expande/colapsa un nodo (grupo o contacto) del árbol de datos.
    ToggleNavNode(String),
    /// Selecciona un nodo del árbol; carta→carga, contenedor→toggle.
    NavClick(String),
    // CRUD del árbol de datos (cosmos-store)
    NewGroup,
    NewContact,
    NewChart,
    DeleteSelected,
    /// Marca el nodo seleccionado para mover (cortar).
    CutNode,
    /// Mueve el nodo cortado bajo el seleccionado (pegar).
    PasteNode,
    RenameStart,
    RenameKey(llimphi_ui::KeyEvent),
    RenameCommit,
    RenameCancel,
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
    // layout guardable (paneles laterales tipo móvil)
    SetNavWidth(f32),
    SetToolsWidth(f32),
    PersistLayout,
    // panel de herramientas (derecha)
    SelectToolCat(ToolCat),
    ToggleToolPanel(ToolPanel),
    // tipo de gráfica del centro
    SetChartView(ChartView),
    /// Resultado del cómputo astronómico PESADO (orto/ocaso/efemérides),
    /// hecho en un worker en vez de bloquear el hilo de UI. `u64` es la
    /// generación: `update` descarta resultados viejos si entretanto se pidió
    /// otro recálculo. `Arc` evita que `Msg: Clone` copie el `AstroState`.
    AstroComputed(u64, std::sync::Arc<crate::astroview::AstroState>),
}

// =====================================================================
// Modelo
// =====================================================================

pub(crate) struct Model {
    pub(crate) chart: Chart,
    pub(crate) overlays: Vec<OverlayKind>,
    pub(crate) harmonic: u32,
    pub(crate) render: RenderModel,
    /// Lecturas astronómicas cacheadas (alt/az, sundial, mareas, orto/ocaso,
    /// eclipses). `None` mientras el worker las calcula —la UI pinta
    /// "calculando…" en vez de bloquearse—. El cómputo (caro: 144 muestras ×
    /// 10 cuerpos) corre SIEMPRE fuera del hilo de UI.
    pub(crate) astro: Option<AstroState>,
    /// `astro` está sucio y hay que recalcularlo. Lo marca `recompute_astro`
    /// dentro de `update`; el despacho al worker ocurre al final de `update`
    /// (que tiene el Handle). La generación evita que un resultado tardío pise
    /// a uno más nuevo.
    pub(crate) astro_dirty: bool,
    pub(crate) astro_gen: u64,
    pub(crate) corpus: Corpus,
    pub(crate) cfg: CosmosConfig,
    pub(crate) theme: Theme,
    pub(crate) error: Option<String>,
    /// Nota efímera en la barra de estado (confirmaciones, "acerca de").
    pub(crate) status_note: Option<String>,
    // multi-carta (tabs del centro)
    pub(crate) open: Vec<OpenTab>,
    pub(crate) active_tab: usize,
    /// `true` = mosaico (todas las cartas lado a lado); `false` = pestañas.
    pub(crate) tile_mode: bool,
    pub(crate) selected_card: Option<String>,
    pub(crate) selected_body: Option<String>,
    // árbol de datos (cosmos-store)
    pub(crate) store: Option<Store>,
    pub(crate) nav_nodes: Vec<NavNode>,
    pub(crate) nav_expanded: HashSet<String>,
    /// Nodo seleccionado en el árbol (clave de [`NavNode`]).
    pub(crate) nav_selected: Option<String>,
    /// Clave del nodo en edición de nombre (`None` = no se renombra).
    pub(crate) nav_rename: Option<String>,
    pub(crate) rename_input: TextInputState,
    /// Clave del nodo cortado, pendiente de pegar (mover).
    pub(crate) nav_cut: Option<String>,
    // layout guardable (3 zonas resizables)
    pub(crate) nav_w: f32,
    pub(crate) tools_w: f32,
    pub(crate) nav_open: bool,
    pub(crate) tools_open: bool,
    // centro + herramientas
    pub(crate) chart_view: ChartView,
    pub(crate) tool_cat: ToolCat,
    pub(crate) expanded_panels: Vec<ToolPanel>,
    // chrome
    pub(crate) menu_open: Option<MenuKind>,
    pub(crate) ctx_open: Option<(f32, f32)>,
    // watchers
    pub(crate) _wawa_watcher: Option<wawa_config::ConfigWatcher>,
    pub(crate) _chart_watcher: Option<notify::RecommendedWatcher>,
}

impl Model {
    /// Etiqueta de la carta activa (para la barra de estado).
    pub(crate) fn active_label(&self) -> &str {
        self.open
            .get(self.active_tab)
            .map(|t| t.label())
            .unwrap_or("—")
    }

    pub(crate) fn toggle_nav(&mut self, key: String) {
        if !self.nav_expanded.remove(&key) {
            self.nav_expanded.insert(key);
        }
    }

    /// El nodo actualmente seleccionado en el árbol, si existe.
    pub(crate) fn selected_node(&self) -> Option<&NavNode> {
        let key = self.nav_selected.as_deref()?;
        self.nav_nodes.iter().find(|n| n.key == key)
    }

    /// Busca un nodo por su clave.
    pub(crate) fn node(&self, key: &str) -> Option<&NavNode> {
        self.nav_nodes.iter().find(|n| n.key == key)
    }

    pub(crate) fn panel_expanded(&self, p: ToolPanel) -> bool {
        self.expanded_panels.contains(&p)
    }

    pub(crate) fn toggle_panel(&mut self, p: ToolPanel) {
        if let Some(i) = self.expanded_panels.iter().position(|x| *x == p) {
            self.expanded_panels.remove(i);
        } else {
            self.expanded_panels.push(p);
        }
    }

    pub(crate) fn nudge_nav(&mut self, dx: f32) {
        self.nav_w = (self.nav_w + dx).clamp(NAV_MIN, NAV_MAX);
    }

    /// El divisor entre centro y herramientas: arrastrar a la derecha
    /// (dx>0) achica el panel de herramientas.
    pub(crate) fn nudge_tools(&mut self, dx: f32) {
        self.tools_w = (self.tools_w - dx).clamp(TOOLS_MIN, TOOLS_MAX);
    }
}
