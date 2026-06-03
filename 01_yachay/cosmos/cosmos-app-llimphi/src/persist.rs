//! Persistencia en disco: estado de UI (`cosmos-ui.json`), carta cargada
//! (`cosmos-chart.json`) con su watcher, y la librería multi-archivo de
//! cartas en el subdirectorio `cosmos-charts/`.

use std::path::PathBuf;

use cosmos_model::{Chart, ChartId, ChartKind, ContactId, StoredBirthData, StoredChartConfig};
use llimphi_ui::Handle;
use serde::{Deserialize, Serialize};

use crate::model::{
    ChartView, CosmosConfig, Msg, OverlayKind, ToolCat, ToolPanel, NAV_WIDTH, TOOLS_WIDTH,
};

/// Subdirectorio dentro del config dir donde viven las cartas guardadas
/// como archivos individuales `<nombre>.json`. El usuario lo gestiona
/// con su file manager — la app solo lista, lee y escribe.
const CHARTS_SUBDIR: &str = "cosmos-charts";

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
pub(crate) struct UiState {
    #[serde(default = "default_overlays")]
    pub(crate) overlays: Vec<OverlayKind>,
    #[serde(default = "default_harmonic")]
    pub(crate) harmonic: u32,
    #[serde(default)]
    pub(crate) cfg: CosmosConfig,
    // layout guardable (paneles laterales tipo móvil)
    #[serde(default = "default_nav_w")]
    pub(crate) nav_w: f32,
    #[serde(default = "default_tools_w")]
    pub(crate) tools_w: f32,
    #[serde(default = "default_true")]
    pub(crate) nav_open: bool,
    #[serde(default = "default_true")]
    pub(crate) tools_open: bool,
    #[serde(default)]
    pub(crate) chart_view: ChartView,
    #[serde(default)]
    pub(crate) tool_cat: ToolCat,
    #[serde(default = "ToolPanel::defaults_expanded")]
    pub(crate) expanded_panels: Vec<ToolPanel>,
    #[serde(default)]
    pub(crate) tile_mode: bool,
    // dock (reparto de paneles por sidebar)
    #[serde(default = "crate::model::default_dock_left")]
    pub(crate) dock_left: Vec<crate::model::DockItem>,
    #[serde(default = "crate::model::default_dock_right")]
    pub(crate) dock_right: Vec<crate::model::DockItem>,
    #[serde(default = "default_yaw")]
    pub(crate) sphere_yaw: f32,
    #[serde(default = "default_pitch")]
    pub(crate) sphere_pitch: f32,
    /// Cielo: `false` = mira al cénit (cielo visible), `true` = mira al
    /// nadir (el hemisferio bajo el horizonte).
    #[serde(default)]
    pub(crate) sky_nadir: bool,
}

fn default_harmonic() -> u32 {
    1
}

fn default_yaw() -> f32 {
    26.0
}

fn default_pitch() -> f32 {
    -64.0
}

/// Topocéntrico activo por default — habilita la tabla de aspectos
/// topocéntricos que el usuario quiere ver de entrada.
fn default_overlays() -> Vec<OverlayKind> {
    vec![OverlayKind::Topocentric]
}

fn default_nav_w() -> f32 {
    NAV_WIDTH
}

fn default_tools_w() -> f32 {
    TOOLS_WIDTH
}

fn default_true() -> bool {
    true
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            overlays: default_overlays(),
            harmonic: 1,
            cfg: CosmosConfig::default(),
            nav_w: NAV_WIDTH,
            tools_w: TOOLS_WIDTH,
            nav_open: true,
            tools_open: true,
            chart_view: ChartView::default(),
            tool_cat: ToolCat::default(),
            expanded_panels: ToolPanel::defaults_expanded(),
            tile_mode: false,
            dock_left: crate::model::default_dock_left(),
            dock_right: crate::model::default_dock_right(),
            sphere_yaw: default_yaw(),
            sphere_pitch: default_pitch(),
            sky_nadir: false,
        }
    }
}

fn ui_state_path() -> Option<PathBuf> {
    wawa_config::config_dir().map(|d| d.join(UI_STATE_FILE))
}

pub(crate) fn chart_path() -> Option<PathBuf> {
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

pub(crate) fn load_chart_from_disk() -> Option<Chart> {
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
pub(crate) fn spawn_chart_watcher(handle: &Handle<Msg>) -> Option<notify::RecommendedWatcher> {
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

pub(crate) fn charts_dir() -> Option<PathBuf> {
    wawa_config::config_dir().map(|d| d.join(CHARTS_SUBDIR))
}

/// Lista los nombres de las cartas guardadas (sin `.json`), ordenados
/// alfabéticamente. Lee el directorio en cada call — barato porque son
/// pocos archivos y la app no es hot-path.
pub(crate) fn list_cards() -> Vec<String> {
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

pub(crate) fn load_card(name: &str) -> Option<Chart> {
    let path = charts_dir()?.join(format!("{name}.json"));
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice::<ChartFile>(&bytes)
        .map_err(|e| eprintln!("cosmos · load_card({name}): {e}"))
        .ok()
        .map(|f| f.into_chart())
}

/// Elimina el archivo de una carta de la biblioteca. No toca la carta
/// cargada (`cosmos-chart.json`).
pub(crate) fn save_chart_to_disk(chart: &Chart) {
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

pub(crate) fn load_ui_state() -> UiState {
    let Some(path) = ui_state_path() else {
        return UiState::default();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return UiState::default();
    };
    match serde_json::from_slice::<UiState>(&bytes) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cosmos · ui-state: no se pudo parsear {path:?}: {e}");
            UiState::default()
        }
    }
}

pub(crate) fn save_ui_state(s: &UiState) {
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
