//! `cosmos_app-tree` — explorador jerárquico Groups → Contacts → Charts.
//!
//! Envuelve [`nahual_widget_tree::TreeView`] con la lógica de dominio
//! de Tahuantinsuyu. Los `RowId` codifican el tipo con prefijo:
//!
//! - `g:<ulid>` → Group
//! - `c:<ulid>` → Contact
//! - `h:<ulid>` → Chart
//!
//! ## Fase 2 — CRUD UX
//!
//! - **Right-click** abre un menú contextual cuyas opciones dependen
//!   del target (raíz, group, contact o chart).
//! - **Renombrar** y **crear** abren un modal con un `TextInput`.
//! - **Crear carta** abre un formulario con los campos mínimos de
//!   `StoredBirthData` (year/month/day/hour/min/tz/lat/lon).
//! - **Borrar** pide confirmación con `window.prompt`.
//!
//! El host (la app) se suscribe a [`TreeEvent`] y traduce a `AppEvent`
//! del bus de nahual para que el canvas/panel reaccionen.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use std::collections::HashSet;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, IntoElement, Pixels, Point, PromptLevel, Render,
    SharedString, Window, div, hsla, prelude::*, px,
};

use cosmos_model::{
    ChartId, ChartKind, ContactId, FreeChartId, GroupId, StoredBirthData, StoredChartConfig,
    TimeCertainty, TreeSelection,
};
use cosmos_store::Store;
use nahual_theme::Theme;
use nahual_widget_text_input::{TextInput, TextInputEvent};
use nahual_widget_tree::{RowId, RowKind, TreeEvent as InnerTreeEvent, TreeRow, TreeView};

const PREFIX_GROUP: &str = "g:";
const PREFIX_CONTACT: &str = "c:";
const PREFIX_CHART: &str = "h:";
/// Prefijo de IDs de filas que representan cartas libres.
const PREFIX_FREE: &str = "f:";
/// IDs sentinela para los nodos virtuales fijos del tree.
const ROW_GENERAL: &str = "general";
const ROW_FREE_ROOT: &str = "free-root";

// =====================================================================
// Eventos públicos
// =====================================================================

#[derive(Clone, Debug)]
pub enum TreeEvent {
    Selected(TreeSelection),
    Opened(TreeSelection),
    /// Una mutación de la jerarquía aconteció (crear, borrar, renombrar).
    /// El host puede usarlo para invalidar caches en otros widgets.
    HierarchyChanged,
    /// El usuario pidió crear una carta libre nueva. El shell la
    /// agrega a su mapa, le da un id efímero, y llama
    /// `set_free_charts` con la lista actualizada.
    NewFreeChartRequested,
    /// El usuario pidió guardar una carta libre como `Chart`
    /// persistido. El shell abre su propio modal con dropdown de
    /// contacto + input de nombre y al confirmar invoca
    /// `store.create_chart`.
    SaveFreeChartRequested(FreeChartId),
    /// Borrar una carta libre del mapa del shell. Si es `sky-now`,
    /// el shell ignora (no se puede borrar el Cielo).
    DeleteFreeChartRequested(FreeChartId),
    /// Submit del modal "Guardar como" — el shell crea/usa el
    /// contacto y persiste la carta. Si `contact` es `None`, el
    /// shell crea uno nuevo con `new_contact_name`.
    FreeChartSaveConfirmed {
        source_id: FreeChartId,
        chart_name: String,
        contact: Option<ContactId>,
        new_contact_name: Option<String>,
    },
    /// Submit del modal "Editar datos" para una carta libre. El
    /// shell aplica al mapa `free_charts` y re-renderea el wheel.
    FreeChartEditConfirmed {
        source_id: FreeChartId,
        birth_data: StoredBirthData,
        label: String,
    },
}

// =====================================================================
// Estado interno
// =====================================================================

/// Target del menú contextual / acciones.
#[derive(Clone, Debug)]
enum MenuTarget {
    Root,
    Group(GroupId),
    Contact(ContactId),
    Chart(ChartId),
    /// Branch "Cartas libres" — menú con "Nueva carta libre".
    FreeChartsRoot,
    /// Una carta libre concreta — menú con "Guardar como…",
    /// "Renombrar" y "Borrar" (salvo `sky-now`, que no se borra).
    FreeChart(FreeChartId),
}

impl MenuTarget {
    fn from_selection(sel: &TreeSelection) -> Self {
        match sel {
            TreeSelection::Group(id) => MenuTarget::Group(*id),
            TreeSelection::Contact(id) => MenuTarget::Contact(*id),
            TreeSelection::Chart(id) => MenuTarget::Chart(*id),
            // "General" comparte menu con Root — el target lógico es
            // crear contactos sin grupo padre.
            TreeSelection::GeneralRoot => MenuTarget::Root,
            TreeSelection::FreeChart(id) => MenuTarget::FreeChart(id.clone()),
            TreeSelection::FreeChartsRoot => MenuTarget::FreeChartsRoot,
        }
    }
}

#[derive(Clone, Debug)]
struct MenuState {
    target: MenuTarget,
    position: Point<Pixels>,
}

/// Modal flotante. Una sola `Modal` activa a la vez — la app no
/// soporta editar varias cosas en simultáneo.
enum Modal {
    RenameGroup {
        id: GroupId,
        input: Entity<TextInput>,
    },
    RenameContact {
        id: ContactId,
        input: Entity<TextInput>,
    },
    RenameChart {
        id: ChartId,
        input: Entity<TextInput>,
    },
    CreateGroup {
        parent: Option<GroupId>,
        input: Entity<TextInput>,
    },
    CreateContact {
        group: Option<GroupId>,
        input: Entity<TextInput>,
    },
    CreateChart {
        contact: ContactId,
        form: ChartForm,
        error: Option<SharedString>,
    },
    /// Editar una carta existente — reusa `ChartForm` pre-cargada.
    /// El submit llama `store.update_chart(id, ...)` preservando
    /// `chart.contact_id`, `related_chart_id`, `module_state` y el
    /// historial.
    EditChart {
        id: ChartId,
        form: ChartForm,
        error: Option<SharedString>,
    },
    /// Editar los datos (fecha/hora/lugar) de una carta libre.
    /// Reusa el mismo `ChartForm` que `EditChart`. El submit emite
    /// `FreeChartEditConfirmed` que el shell aplica al mapa
    /// `free_charts` y re-renderea el wheel.
    EditFreeChart {
        source_id: FreeChartId,
        form: ChartForm,
        error: Option<SharedString>,
    },
    /// Guardar una carta libre como `Chart` persistido. El usuario
    /// elige nombre + contacto destino (existente de la lista o
    /// uno nuevo creado al vuelo). El shell escucha
    /// `TreeEvent::FreeChartSaveConfirmed` y materializa.
    SaveFreeChart {
        source_id: FreeChartId,
        name: Entity<TextInput>,
        /// Nombre del contacto NUEVO (solo aplica si
        /// `selected_contact == None`). Vacío para reusar uno
        /// existente.
        new_contact_name: Entity<TextInput>,
        /// `Some(id)` = usar contacto existente; `None` = crear
        /// contacto nuevo con `new_contact_name`.
        selected_contact: Option<ContactId>,
        /// Snapshot de contactos visibles al usuario en el momento
        /// de abrir el modal. Incluye contact id + label (nombre).
        all_contacts: Vec<(ContactId, String)>,
        error: Option<SharedString>,
    },
}

struct ChartForm {
    name: Entity<TextInput>,
    place: Entity<TextInput>,
    year: Entity<TextInput>,
    month: Entity<TextInput>,
    day: Entity<TextInput>,
    hour: Entity<TextInput>,
    minute: Entity<TextInput>,
    tz_offset_min: Entity<TextInput>,
    lat: Entity<TextInput>,
    lon: Entity<TextInput>,
    alt: Entity<TextInput>,
}

// =====================================================================
// Widget
// =====================================================================

pub struct TahuantinsuyuTree {
    store: Store,
    inner: Entity<TreeView>,
    expanded: HashSet<String>,
    menu: Option<MenuState>,
    modal: Option<Modal>,
    /// `true` cuando el dropdown de "ciudad rápida" en el ChartForm
    /// está abierto. Vive en el tree (no en ChartForm) porque las
    /// closures de los click handlers necesitan mutarlo via `cx.listener`.
    city_picker_open: bool,
    /// Atlas de ciudades para el dropdown del form. Se inicializa con
    /// `default_city_presets()` (90 ciudades hardcoded). El host puede
    /// llamar [`Self::set_city_atlas`] para reemplazar por uno custom
    /// cargado desde disco (TSV).
    city_atlas: Vec<CityPreset>,
    /// Filtro de búsqueda activo. Vacío = sin filtro (jerarquía
    /// completa). Cuando hay texto, refresh() solo incluye rows cuyo
    /// nombre (group / contact / chart label) contenga el substring
    /// case-insensitive, y auto-expande los ancestros para que el
    /// match sea visible.
    search_filter: String,
    /// TextInput para el filtro — vive arriba del tree.
    search_input: Entity<TextInput>,
    /// Lista de cartas libres a mostrar bajo "Cartas libres". El shell
    /// la actualiza vía [`Self::set_free_charts`] cada vez que crea,
    /// renombra o borra una. El orden de inserción es el de display
    /// (los nuevos van al final; "Cielo ahora" siempre va primero por
    /// convención del shell).
    free_charts: Vec<FreeChartEntry>,
}

/// Entrada de la sección "Cartas libres" — id + label visible +
/// birth_data clonado (para pre-poblar el modal "Editar datos…").
/// El Chart completo vive en el shell; el tree mantiene esta
/// proyección compacta para no depender del shell en cada operación.
#[derive(Clone, Debug)]
pub struct FreeChartEntry {
    pub id: FreeChartId,
    pub label: String,
    pub birth_data: StoredBirthData,
}

/// Preset de ciudad con datos canónicos para autocompletar lat/lon/tz
/// al elegirlo en el form. TZ es la zona estándar **sin DST** — el
/// usuario afina si necesita. `name` es `String` (no &'static) para
/// permitir cargar atlas custom desde disco vía
/// [`TahuantinsuyuTree::set_city_atlas`].
#[derive(Clone, Debug)]
pub struct CityPreset {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub tz_offset_minutes: i32,
}

/// Atlas hardcoded — 90 ciudades canónicas que cubren la mayoría de
/// casos de uso. El usuario puede sobrescribirlas pasando un atlas
/// custom vía [`TahuantinsuyuTree::set_city_atlas`] (típicamente
/// cargado desde `$XDG_DATA_HOME/cosmos_app/atlas.tsv`).
pub fn default_city_presets() -> Vec<CityPreset> {
    vec![
        // Latinoamérica
        CityPreset { name: "Buenos Aires, AR".into(), lat: -34.6037, lon: -58.3816, tz_offset_minutes: -180 },
    CityPreset { name: "Córdoba, AR".into(),      lat: -31.4201, lon: -64.1888, tz_offset_minutes: -180 },
    CityPreset { name: "Rosario, AR".into(),      lat: -32.9587, lon: -60.6930, tz_offset_minutes: -180 },
    CityPreset { name: "Mendoza, AR".into(),      lat: -32.8908, lon: -68.8272, tz_offset_minutes: -180 },
    CityPreset { name: "Caracas, VE".into(),      lat: 10.4806,  lon: -66.9036, tz_offset_minutes: -240 },
    CityPreset { name: "Maracaibo, VE".into(),    lat: 10.6427,  lon: -71.6125, tz_offset_minutes: -240 },
    CityPreset { name: "Valencia, VE".into(),     lat: 10.1620,  lon: -68.0078, tz_offset_minutes: -240 },
    CityPreset { name: "Bogotá, CO".into(),       lat: 4.7110,   lon: -74.0721, tz_offset_minutes: -300 },
    CityPreset { name: "Medellín, CO".into(),     lat: 6.2442,   lon: -75.5812, tz_offset_minutes: -300 },
    CityPreset { name: "Cali, CO".into(),         lat: 3.4516,   lon: -76.5320, tz_offset_minutes: -300 },
    CityPreset { name: "Lima, PE".into(),         lat: -12.0464, lon: -77.0428, tz_offset_minutes: -300 },
    CityPreset { name: "Cusco, PE".into(),        lat: -13.5319, lon: -71.9675, tz_offset_minutes: -300 },
    CityPreset { name: "Santiago, CL".into(),     lat: -33.4489, lon: -70.6693, tz_offset_minutes: -240 },
    CityPreset { name: "Valparaíso, CL".into(),   lat: -33.0472, lon: -71.6127, tz_offset_minutes: -240 },
    CityPreset { name: "Quito, EC".into(),        lat: -0.1807,  lon: -78.4678, tz_offset_minutes: -300 },
    CityPreset { name: "Guayaquil, EC".into(),    lat: -2.1709,  lon: -79.9224, tz_offset_minutes: -300 },
    CityPreset { name: "Montevideo, UY".into(),   lat: -34.9011, lon: -56.1645, tz_offset_minutes: -180 },
    CityPreset { name: "Asunción, PY".into(),     lat: -25.2637, lon: -57.5759, tz_offset_minutes: -240 },
    CityPreset { name: "La Paz, BO".into(),       lat: -16.4897, lon: -68.1193, tz_offset_minutes: -240 },
    CityPreset { name: "Ciudad de México".into(), lat: 19.4326,  lon: -99.1332, tz_offset_minutes: -360 },
    CityPreset { name: "Guadalajara, MX".into(),  lat: 20.6597,  lon: -103.3496, tz_offset_minutes: -360 },
    CityPreset { name: "Monterrey, MX".into(),    lat: 25.6866,  lon: -100.3161, tz_offset_minutes: -360 },
    CityPreset { name: "Habana, CU".into(),       lat: 23.1136,  lon: -82.3666, tz_offset_minutes: -300 },
    CityPreset { name: "San Juan, PR".into(),     lat: 18.4655,  lon: -66.1057, tz_offset_minutes: -240 },
    CityPreset { name: "San José, CR".into(),     lat: 9.9281,   lon: -84.0907, tz_offset_minutes: -360 },
    CityPreset { name: "Panamá, PA".into(),       lat: 8.9824,   lon: -79.5199, tz_offset_minutes: -300 },
    CityPreset { name: "San Salvador, SV".into(), lat: 13.6929,  lon: -89.2182, tz_offset_minutes: -360 },
    CityPreset { name: "Guatemala, GT".into(),    lat: 14.6349,  lon: -90.5069, tz_offset_minutes: -360 },
    CityPreset { name: "Tegucigalpa, HN".into(),  lat: 14.0723,  lon: -87.1921, tz_offset_minutes: -360 },
    CityPreset { name: "Managua, NI".into(),      lat: 12.1149,  lon: -86.2362, tz_offset_minutes: -360 },
    CityPreset { name: "Santo Domingo, DO".into(), lat: 18.4861, lon: -69.9312, tz_offset_minutes: -240 },
    CityPreset { name: "São Paulo, BR".into(),    lat: -23.5505, lon: -46.6333, tz_offset_minutes: -180 },
    CityPreset { name: "Rio de Janeiro, BR".into(), lat: -22.9068, lon: -43.1729, tz_offset_minutes: -180 },
    CityPreset { name: "Brasília, BR".into(),     lat: -15.8267, lon: -47.9218, tz_offset_minutes: -180 },
    CityPreset { name: "Salvador, BR".into(),     lat: -12.9777, lon: -38.5016, tz_offset_minutes: -180 },
    // España
    CityPreset { name: "Madrid, ES".into(),       lat: 40.4168,  lon: -3.7038,  tz_offset_minutes: 60 },
    CityPreset { name: "Barcelona, ES".into(),    lat: 41.3851,  lon: 2.1734,   tz_offset_minutes: 60 },
    CityPreset { name: "Sevilla, ES".into(),      lat: 37.3891,  lon: -5.9845,  tz_offset_minutes: 60 },
    CityPreset { name: "Valencia, ES".into(),     lat: 39.4699,  lon: -0.3763,  tz_offset_minutes: 60 },
    CityPreset { name: "Bilbao, ES".into(),       lat: 43.2630,  lon: -2.9350,  tz_offset_minutes: 60 },
    // Europa
    CityPreset { name: "London, UK".into(),       lat: 51.5074,  lon: -0.1278,  tz_offset_minutes: 0 },
    CityPreset { name: "Paris, FR".into(),        lat: 48.8566,  lon: 2.3522,   tz_offset_minutes: 60 },
    CityPreset { name: "Berlin, DE".into(),       lat: 52.5200,  lon: 13.4050,  tz_offset_minutes: 60 },
    CityPreset { name: "München, DE".into(),      lat: 48.1351,  lon: 11.5820,  tz_offset_minutes: 60 },
    CityPreset { name: "Roma, IT".into(),         lat: 41.9028,  lon: 12.4964,  tz_offset_minutes: 60 },
    CityPreset { name: "Milano, IT".into(),       lat: 45.4642,  lon: 9.1900,   tz_offset_minutes: 60 },
    CityPreset { name: "Amsterdam, NL".into(),    lat: 52.3676,  lon: 4.9041,   tz_offset_minutes: 60 },
    CityPreset { name: "Bruxelles, BE".into(),    lat: 50.8503,  lon: 4.3517,   tz_offset_minutes: 60 },
    CityPreset { name: "Wien, AT".into(),         lat: 48.2082,  lon: 16.3738,  tz_offset_minutes: 60 },
    CityPreset { name: "Zürich, CH".into(),       lat: 47.3769,  lon: 8.5417,   tz_offset_minutes: 60 },
    CityPreset { name: "Lisboa, PT".into(),       lat: 38.7223,  lon: -9.1393,  tz_offset_minutes: 0 },
    CityPreset { name: "Dublin, IE".into(),       lat: 53.3498,  lon: -6.2603,  tz_offset_minutes: 0 },
    CityPreset { name: "Stockholm, SE".into(),    lat: 59.3293,  lon: 18.0686,  tz_offset_minutes: 60 },
    CityPreset { name: "Oslo, NO".into(),         lat: 59.9139,  lon: 10.7522,  tz_offset_minutes: 60 },
    CityPreset { name: "København, DK".into(),    lat: 55.6761,  lon: 12.5683,  tz_offset_minutes: 60 },
    CityPreset { name: "Helsinki, FI".into(),     lat: 60.1699,  lon: 24.9384,  tz_offset_minutes: 120 },
    CityPreset { name: "Warszawa, PL".into(),     lat: 52.2297,  lon: 21.0122,  tz_offset_minutes: 60 },
    CityPreset { name: "Praha, CZ".into(),        lat: 50.0755,  lon: 14.4378,  tz_offset_minutes: 60 },
    CityPreset { name: "Budapest, HU".into(),     lat: 47.4979,  lon: 19.0402,  tz_offset_minutes: 60 },
    CityPreset { name: "Athina, GR".into(),       lat: 37.9838,  lon: 23.7275,  tz_offset_minutes: 120 },
    CityPreset { name: "İstanbul, TR".into(),     lat: 41.0082,  lon: 28.9784,  tz_offset_minutes: 180 },
    CityPreset { name: "Moskva, RU".into(),       lat: 55.7558,  lon: 37.6173,  tz_offset_minutes: 180 },
    // USA + Canada
    CityPreset { name: "New York, US".into(),     lat: 40.7128,  lon: -74.0060, tz_offset_minutes: -300 },
    CityPreset { name: "Los Angeles, US".into(),  lat: 34.0522,  lon: -118.2437, tz_offset_minutes: -480 },
    CityPreset { name: "Chicago, US".into(),      lat: 41.8781,  lon: -87.6298, tz_offset_minutes: -360 },
    CityPreset { name: "Miami, US".into(),        lat: 25.7617,  lon: -80.1918, tz_offset_minutes: -300 },
    CityPreset { name: "Houston, US".into(),      lat: 29.7604,  lon: -95.3698, tz_offset_minutes: -360 },
    CityPreset { name: "San Francisco, US".into(), lat: 37.7749, lon: -122.4194, tz_offset_minutes: -480 },
    CityPreset { name: "Seattle, US".into(),      lat: 47.6062,  lon: -122.3321, tz_offset_minutes: -480 },
    CityPreset { name: "Boston, US".into(),       lat: 42.3601,  lon: -71.0589, tz_offset_minutes: -300 },
    CityPreset { name: "Washington DC".into(),    lat: 38.9072,  lon: -77.0369, tz_offset_minutes: -300 },
    CityPreset { name: "Toronto, CA".into(),      lat: 43.6532,  lon: -79.3832, tz_offset_minutes: -300 },
    CityPreset { name: "Montreal, CA".into(),     lat: 45.5017,  lon: -73.5673, tz_offset_minutes: -300 },
    CityPreset { name: "Vancouver, CA".into(),    lat: 49.2827,  lon: -123.1207, tz_offset_minutes: -480 },
    // Asia
    CityPreset { name: "Tokyo, JP".into(),        lat: 35.6762,  lon: 139.6503, tz_offset_minutes: 540 },
    CityPreset { name: "Beijing, CN".into(),      lat: 39.9042,  lon: 116.4074, tz_offset_minutes: 480 },
    CityPreset { name: "Shanghai, CN".into(),     lat: 31.2304,  lon: 121.4737, tz_offset_minutes: 480 },
    CityPreset { name: "Hong Kong".into(),        lat: 22.3193,  lon: 114.1694, tz_offset_minutes: 480 },
    CityPreset { name: "Singapore".into(),        lat: 1.3521,   lon: 103.8198, tz_offset_minutes: 480 },
    CityPreset { name: "Seoul, KR".into(),        lat: 37.5665,  lon: 126.9780, tz_offset_minutes: 540 },
    CityPreset { name: "Bangkok, TH".into(),      lat: 13.7563,  lon: 100.5018, tz_offset_minutes: 420 },
    CityPreset { name: "Jakarta, ID".into(),      lat: -6.2088,  lon: 106.8456, tz_offset_minutes: 420 },
    CityPreset { name: "Manila, PH".into(),       lat: 14.5995,  lon: 120.9842, tz_offset_minutes: 480 },
    CityPreset { name: "Mumbai, IN".into(),       lat: 19.0760,  lon: 72.8777,  tz_offset_minutes: 330 },
    CityPreset { name: "Delhi, IN".into(),        lat: 28.7041,  lon: 77.1025,  tz_offset_minutes: 330 },
    CityPreset { name: "Bangalore, IN".into(),    lat: 12.9716,  lon: 77.5946,  tz_offset_minutes: 330 },
    CityPreset { name: "Karachi, PK".into(),      lat: 24.8607,  lon: 67.0011,  tz_offset_minutes: 300 },
    CityPreset { name: "Tehran, IR".into(),       lat: 35.6892,  lon: 51.3890,  tz_offset_minutes: 210 },
    CityPreset { name: "Dubai, AE".into(),        lat: 25.2048,  lon: 55.2708,  tz_offset_minutes: 240 },
    CityPreset { name: "Tel Aviv, IL".into(),     lat: 32.0853,  lon: 34.7818,  tz_offset_minutes: 120 },
    // África
    CityPreset { name: "Cairo, EG".into(),        lat: 30.0444,  lon: 31.2357,  tz_offset_minutes: 120 },
    CityPreset { name: "Lagos, NG".into(),        lat: 6.5244,   lon: 3.3792,   tz_offset_minutes: 60 },
    CityPreset { name: "Nairobi, KE".into(),      lat: -1.2921,  lon: 36.8219,  tz_offset_minutes: 180 },
    CityPreset { name: "Johannesburg, ZA".into(), lat: -26.2041, lon: 28.0473,  tz_offset_minutes: 120 },
    CityPreset { name: "Cape Town, ZA".into(),    lat: -33.9249, lon: 18.4241,  tz_offset_minutes: 120 },
    CityPreset { name: "Casablanca, MA".into(),   lat: 33.5731,  lon: -7.5898,  tz_offset_minutes: 60 },
    // Oceanía
    CityPreset { name: "Sydney, AU".into(),       lat: -33.8688, lon: 151.2093, tz_offset_minutes: 600 },
    CityPreset { name: "Melbourne, AU".into(),    lat: -37.8136, lon: 144.9631, tz_offset_minutes: 600 },
    CityPreset { name: "Auckland, NZ".into(),     lat: -36.8485, lon: 174.7633, tz_offset_minutes: 720 },
    ]
}

/// Parsea un atlas TSV (tab-separated values): cada línea no vacía y
/// no comentario es `name<TAB>lat<TAB>lon<TAB>tz_offset_minutes`.
/// Devuelve solo las filas válidas — las inválidas se descartan en
/// silencio (no abortamos la carga por una línea mal formada).
pub fn parse_city_atlas_tsv(content: &str) -> Vec<CityPreset> {
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 4 {
            continue;
        }
        let name = parts[0].trim().to_string();
        let lat = parts[1].trim().parse::<f64>();
        let lon = parts[2].trim().parse::<f64>();
        let tz = parts[3].trim().parse::<i32>();
        if let (Ok(lat), Ok(lon), Ok(tz)) = (lat, lon, tz) {
            if !name.is_empty() {
                out.push(CityPreset {
                    name,
                    lat,
                    lon,
                    tz_offset_minutes: tz,
                });
            }
        }
    }
    out
}

impl EventEmitter<TreeEvent> for TahuantinsuyuTree {}

impl TahuantinsuyuTree {
    pub fn new(store: Store, cx: &mut Context<'_, Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();

        let inner = cx.new(|cx| TreeView::new("cosmos_app-tree", cx));
        cx.subscribe(&inner, |this: &mut Self, _, ev, cx| {
            this.on_inner(ev, cx);
        })
        .detach();

        let search_input = cx.new(|cx| {
            TextInput::new(String::new(), cx)
                .with_placeholder(SharedString::from("Buscar nombre…"))
        });
        cx.subscribe(
            &search_input,
            |this: &mut Self, _, ev: &TextInputEvent, cx| match ev {
                TextInputEvent::Confirmed(value) => {
                    this.set_search_filter(value.clone(), cx);
                }
                TextInputEvent::Cancelled => {
                    this.set_search_filter(String::new(), cx);
                }
            },
        )
        .detach();

        let mut me = Self {
            store,
            inner,
            expanded: HashSet::new(),
            menu: None,
            modal: None,
            city_picker_open: false,
            city_atlas: default_city_presets(),
            search_filter: String::new(),
            search_input,
            free_charts: Vec::new(),
        };
        // "Cartas libres" expandida por default — el usuario espera ver
        // "Cielo ahora" sin tener que hacer click en el chevron.
        me.expanded.insert(ROW_FREE_ROOT.to_string());
        me.refresh(cx);
        me
    }

    /// Reemplaza la lista de cartas libres del tree. El shell la llama
    /// cada vez que crea, renombra o borra una carta libre.
    pub fn set_free_charts(&mut self, entries: Vec<FreeChartEntry>, cx: &mut Context<'_, Self>) {
        self.free_charts = entries;
        self.refresh(cx);
    }

    /// Reemplaza el atlas de ciudades del dropdown. La app llama esto
    /// al boot si encuentra un archivo TSV custom en disco.
    pub fn set_city_atlas(&mut self, atlas: Vec<CityPreset>, cx: &mut Context<'_, Self>) {
        if !atlas.is_empty() {
            self.city_atlas = atlas;
            cx.notify();
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<'_, Self>) {
        let mut rows = Vec::new();

        // 1) General — branch fijo al top. Contiene los contactos sin
        //    grupo padre (parent=None). Siempre presente.
        let general_expanded = self.expanded.contains(ROW_GENERAL);
        rows.push(TreeRow {
            id: RowId::new(ROW_GENERAL.to_string()),
            label: "General".to_string(),
            depth: 0,
            kind: RowKind::Branch,
            expanded: general_expanded,
            icon: Some("◇".into()),
        });
        if general_expanded {
            self.append_contacts(None, 1, &mut rows);
        }

        // 2) Groups top-level con sus contenidos.
        self.append_groups(None, 0, &mut rows);

        // 3) Cartas libres — branch fijo al FONDO. Contiene "Cielo
        //    ahora" + cualquier carta libre creada por el usuario.
        //    Permanece visible aún sin entries (el usuario puede
        //    crear nuevas desde su menu contextual).
        let free_expanded = self.expanded.contains(ROW_FREE_ROOT);
        rows.push(TreeRow {
            id: RowId::new(ROW_FREE_ROOT.to_string()),
            label: "Cartas libres".to_string(),
            depth: 0,
            kind: RowKind::Branch,
            expanded: free_expanded,
            icon: Some("🜨".into()),
        });
        if free_expanded {
            for e in &self.free_charts {
                let id_str = format!("{}{}", PREFIX_FREE, e.id.as_str());
                let icon = if e.id.is_sky_now() { "⏱" } else { "✦" };
                rows.push(TreeRow {
                    id: RowId::new(id_str),
                    label: e.label.clone(),
                    depth: 1,
                    kind: RowKind::Leaf,
                    expanded: false,
                    icon: Some(icon.into()),
                });
            }
        }

        self.inner.update(cx, |t, cx| t.set_rows(rows, cx));
    }

    /// Actualiza el filtro de búsqueda — texto vacío = sin filtro.
    /// Cuando hay filtro, expande automáticamente los ancestros que
    /// contienen matches para que el usuario vea los resultados sin
    /// tener que clickear chevrons.
    fn set_search_filter(&mut self, filter: String, cx: &mut Context<'_, Self>) {
        self.search_filter = filter.trim().to_lowercase();
        if !self.search_filter.is_empty() {
            self.auto_expand_matches();
        }
        self.refresh(cx);
    }

    /// Pre-expande todos los groups + contacts que contienen al menos
    /// un descendiente cuyo nombre matchee el filtro. Hace una pasada
    /// recursiva agregando ids al `expanded` set.
    fn auto_expand_matches(&mut self) {
        fn walk_group(this: &mut TahuantinsuyuTree, group_id: GroupId) -> bool {
            let mut any_match = false;
            // Sub-groups recursivamente.
            if let Ok(children) = this.store.list_groups(Some(group_id)) {
                for g in children {
                    let name_match = g.name.to_lowercase().contains(&this.search_filter);
                    let child_match = walk_group(this, g.id);
                    if name_match || child_match {
                        this.expanded.insert(format!("{}{}", PREFIX_GROUP, g.id));
                        any_match = true;
                    }
                }
            }
            // Contacts directos.
            if let Ok(contacts) = this.store.list_contacts(Some(group_id)) {
                for c in contacts {
                    let name_match = c.name.to_lowercase().contains(&this.search_filter);
                    let chart_match = contact_has_matching_chart(this, c.id);
                    if name_match || chart_match {
                        this.expanded.insert(format!("{}{}", PREFIX_CONTACT, c.id));
                        any_match = true;
                    }
                }
            }
            any_match
        }
        fn contact_has_matching_chart(this: &TahuantinsuyuTree, contact_id: ContactId) -> bool {
            this.store
                .list_charts(contact_id)
                .map(|charts| {
                    charts
                        .iter()
                        .any(|h| h.label.to_lowercase().contains(&this.search_filter))
                })
                .unwrap_or(false)
        }

        // Top-level groups + contacts directos en raíz.
        if let Ok(groups) = self.store.list_groups(None) {
            for g in groups {
                let name_match = g.name.to_lowercase().contains(&self.search_filter);
                let child_match = walk_group(self, g.id);
                if name_match || child_match {
                    self.expanded.insert(format!("{}{}", PREFIX_GROUP, g.id));
                }
            }
        }
        if let Ok(contacts) = self.store.list_contacts(None) {
            for c in contacts {
                let name_match = c.name.to_lowercase().contains(&self.search_filter);
                let chart_match = contact_has_matching_chart(self, c.id);
                if name_match || chart_match {
                    self.expanded.insert(format!("{}{}", PREFIX_CONTACT, c.id));
                }
            }
        }
    }

    /// `true` si la jerarquía bajo `group_id` (recursivo) tiene
    /// algún descendiente que matchee el filtro de búsqueda.
    fn group_has_match(&self, group_id: GroupId) -> bool {
        if let Ok(sub) = self.store.list_groups(Some(group_id)) {
            for g in &sub {
                if g.name.to_lowercase().contains(&self.search_filter) {
                    return true;
                }
                if self.group_has_match(g.id) {
                    return true;
                }
            }
        }
        if let Ok(contacts) = self.store.list_contacts(Some(group_id)) {
            for c in &contacts {
                if c.name.to_lowercase().contains(&self.search_filter) {
                    return true;
                }
                if let Ok(charts) = self.store.list_charts(c.id) {
                    if charts
                        .iter()
                        .any(|h| h.label.to_lowercase().contains(&self.search_filter))
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `true` si el contacto tiene una carta cuyo label matchee.
    fn contact_has_match(&self, contact_id: ContactId) -> bool {
        if let Ok(charts) = self.store.list_charts(contact_id) {
            return charts
                .iter()
                .any(|h| h.label.to_lowercase().contains(&self.search_filter));
        }
        false
    }

    fn append_groups(&self, parent: Option<GroupId>, depth: u32, out: &mut Vec<TreeRow>) {
        let groups = match self.store.list_groups(parent) {
            Ok(v) => v,
            Err(_) => return,
        };
        for g in groups {
            // Filtro: incluir si el group matchea por nombre o tiene
            // algún descendiente matching.
            if !self.search_filter.is_empty() {
                let name_match = g.name.to_lowercase().contains(&self.search_filter);
                if !name_match && !self.group_has_match(g.id) {
                    continue;
                }
            }
            let id_str = format!("{}{}", PREFIX_GROUP, g.id);
            let expanded = self.expanded.contains(&id_str);
            out.push(TreeRow {
                id: RowId::new(id_str.clone()),
                label: g.name.clone(),
                depth,
                kind: RowKind::Branch,
                expanded,
                icon: Some("📁".into()),
            });
            if expanded {
                self.append_groups(Some(g.id), depth + 1, out);
                self.append_contacts(Some(g.id), depth + 1, out);
            }
        }
    }

    fn append_contacts(&self, parent: Option<GroupId>, depth: u32, out: &mut Vec<TreeRow>) {
        let contacts = match self.store.list_contacts(parent) {
            Ok(v) => v,
            Err(_) => return,
        };
        for c in contacts {
            if !self.search_filter.is_empty() {
                let name_match = c.name.to_lowercase().contains(&self.search_filter);
                if !name_match && !self.contact_has_match(c.id) {
                    continue;
                }
            }
            let id_str = format!("{}{}", PREFIX_CONTACT, c.id);
            let expanded = self.expanded.contains(&id_str);
            out.push(TreeRow {
                id: RowId::new(id_str.clone()),
                label: c.name.clone(),
                depth,
                kind: RowKind::Branch,
                expanded,
                icon: Some("🜨".into()),
            });
            if expanded {
                self.append_charts(c.id, depth + 1, out);
            }
        }
    }

    fn append_charts(&self, contact: ContactId, depth: u32, out: &mut Vec<TreeRow>) {
        let charts = match self.store.list_charts(contact) {
            Ok(v) => v,
            Err(_) => return,
        };
        for h in charts {
            if !self.search_filter.is_empty()
                && !h.label.to_lowercase().contains(&self.search_filter)
            {
                continue;
            }
            let id_str = format!("{}{}", PREFIX_CHART, h.id);
            out.push(TreeRow {
                id: RowId::new(id_str),
                label: h.label.clone(),
                depth,
                kind: RowKind::Leaf,
                expanded: false,
                icon: Some("✦".into()),
            });
        }
    }

    fn on_inner(&mut self, ev: &InnerTreeEvent, cx: &mut Context<'_, Self>) {
        match ev {
            InnerTreeEvent::ChevronToggled(id) => {
                let s = id.as_str().to_string();
                if !self.expanded.remove(&s) {
                    self.expanded.insert(s);
                }
                self.refresh(cx);
            }
            InnerTreeEvent::RowClicked(id) => {
                if self.menu.is_some() {
                    self.menu = None;
                    cx.notify();
                }
                if let Some(sel) = parse_row(id) {
                    cx.emit(TreeEvent::Selected(sel));
                }
            }
            InnerTreeEvent::RowDoubleClicked(id) => {
                if let Some(sel) = parse_row(id) {
                    cx.emit(TreeEvent::Opened(sel));
                }
            }
            InnerTreeEvent::ContextMenuRequested { id, position } => {
                let target = match id.as_ref().and_then(parse_row) {
                    Some(sel) => MenuTarget::from_selection(&sel),
                    None => MenuTarget::Root,
                };
                self.menu = Some(MenuState {
                    target,
                    position: *position,
                });
                cx.notify();
            }
            InnerTreeEvent::ActiveChanged(_) => {}
        }
    }

    // -----------------------------------------------------------------
    // Acciones del menú
    // -----------------------------------------------------------------

    fn close_menu(&mut self, cx: &mut Context<'_, Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    fn close_modal(&mut self, cx: &mut Context<'_, Self>) {
        if self.modal.take().is_some() {
            self.city_picker_open = false;
            cx.notify();
        }
    }

    fn toggle_city_picker(&mut self, cx: &mut Context<'_, Self>) {
        self.city_picker_open = !self.city_picker_open;
        cx.notify();
    }

    /// Aplica un city preset al ChartForm activo (CreateChart o
    /// EditChart). Setea place, lat, lon, tz_offset_min vía
    /// `TextInput::set_text` y cierra el picker.
    fn apply_city_preset(&mut self, preset: &CityPreset, cx: &mut Context<'_, Self>) {
        let form = match self.modal.as_mut() {
            Some(Modal::CreateChart { form, .. }) => form,
            Some(Modal::EditChart { form, .. }) => form,
            Some(Modal::EditFreeChart { form, .. }) => form,
            _ => {
                self.city_picker_open = false;
                cx.notify();
                return;
            }
        };
        let place = form.place.clone();
        let lat = form.lat.clone();
        let lon = form.lon.clone();
        let tz = form.tz_offset_min.clone();
        let name = preset.name.clone();
        let lat_val = preset.lat;
        let lon_val = preset.lon;
        let tz_val = preset.tz_offset_minutes;
        place.update(cx, |i, cx| i.set_text(name, cx));
        lat.update(cx, |i, cx| i.set_text(format!("{}", lat_val), cx));
        lon.update(cx, |i, cx| i.set_text(format!("{}", lon_val), cx));
        tz.update(cx, |i, cx| i.set_text(tz_val.to_string(), cx));
        self.city_picker_open = false;
        cx.notify();
    }

    fn open_create_group(
        &mut self,
        parent: Option<GroupId>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let input = self.make_input("Nombre del grupo", "", window, cx);
        self.modal = Some(Modal::CreateGroup { parent, input });
        self.close_menu(cx);
    }

    fn open_create_contact(
        &mut self,
        group: Option<GroupId>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let input = self.make_input("Nombre del contacto", "", window, cx);
        self.modal = Some(Modal::CreateContact { group, input });
        self.close_menu(cx);
    }

    fn open_edit_chart(
        &mut self,
        id: ChartId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        // Cargar la carta existente; si no se puede, fallamos en silencio.
        let chart = match self.store.get_chart(id) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[tree] open_edit_chart {}: {}", id, e);
                return;
            }
        };
        let bd = &chart.birth_data;
        let form = ChartForm {
            name: self.make_input("Etiqueta de la carta", &chart.label, window, cx),
            place: self.make_input(
                "Lugar (ciudad, país)",
                bd.birthplace_label.as_deref().unwrap_or(""),
                window,
                cx,
            ),
            year: self.make_input("Año", &bd.year.to_string(), window, cx),
            month: self.make_input("Mes", &bd.month.to_string(), window, cx),
            day: self.make_input("Día", &bd.day.to_string(), window, cx),
            hour: self.make_input("Hora (0-23)", &bd.hour.to_string(), window, cx),
            minute: self.make_input("Minuto", &bd.minute.to_string(), window, cx),
            tz_offset_min: self.make_input(
                "TZ offset (min)",
                &bd.tz_offset_minutes.to_string(),
                window,
                cx,
            ),
            lat: self.make_input("Latitud (°)", &format!("{}", bd.latitude_deg), window, cx),
            lon: self.make_input(
                "Longitud (°)",
                &format!("{}", bd.longitude_deg),
                window,
                cx,
            ),
            alt: self.make_input("Altitud (m)", &format!("{}", bd.altitude_m), window, cx),
        };
        form.name.update(cx, |i, _| i.request_focus(window));
        self.modal = Some(Modal::EditChart {
            id,
            form,
            error: None,
        });
        self.close_menu(cx);
    }

    /// Abre el modal "Guardar como" para una carta libre. Pre-puebla
    /// el `name` con el label actual de la entry. La lista de
    /// contactos es un snapshot recursivo de toda la jerarquía
    /// (no solo el nivel raíz). El usuario elige uno existente o
    /// deja en "Nuevo contacto" para que se cree uno al confirmar.
    /// Abre el modal "Editar datos" para una carta libre. Pre-puebla
    /// `ChartForm` con `birth_data` actual de la entry. Submit emite
    /// `FreeChartEditConfirmed` que el shell aplica al mapa de
    /// `free_charts` y re-renderea.
    fn open_edit_free_chart_modal(
        &mut self,
        source_id: FreeChartId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let entry = match self.free_charts.iter().find(|e| e.id == source_id) {
            Some(e) => e.clone(),
            None => return,
        };
        let bd = &entry.birth_data;
        let form = ChartForm {
            name: self.make_input("Etiqueta de la carta", &entry.label, window, cx),
            place: self.make_input(
                "Lugar (ciudad, país)",
                bd.birthplace_label.as_deref().unwrap_or(""),
                window,
                cx,
            ),
            year: self.make_input("Año", &bd.year.to_string(), window, cx),
            month: self.make_input("Mes", &bd.month.to_string(), window, cx),
            day: self.make_input("Día", &bd.day.to_string(), window, cx),
            hour: self.make_input("Hora (0-23)", &bd.hour.to_string(), window, cx),
            minute: self.make_input("Minuto", &bd.minute.to_string(), window, cx),
            tz_offset_min: self.make_input(
                "TZ offset (min)",
                &bd.tz_offset_minutes.to_string(),
                window,
                cx,
            ),
            lat: self.make_input("Latitud (°)", &format!("{}", bd.latitude_deg), window, cx),
            lon: self.make_input(
                "Longitud (°)",
                &format!("{}", bd.longitude_deg),
                window,
                cx,
            ),
            alt: self.make_input("Altitud (m)", &format!("{}", bd.altitude_m), window, cx),
        };
        form.name.update(cx, |i, _| i.request_focus(window));
        self.modal = Some(Modal::EditFreeChart {
            source_id,
            form,
            error: None,
        });
        self.close_menu(cx);
    }

    /// Cambia `selected_contact` del modal `SaveFreeChart` activo
    /// sin recrear los inputs. Permite alternar entre los botones
    /// radio "contacto existente" y "Nuevo contacto…".
    fn set_save_modal_contact(
        &mut self,
        new_selection: Option<ContactId>,
        expected_source: &FreeChartId,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(Modal::SaveFreeChart {
            source_id,
            selected_contact,
            ..
        }) = self.modal.as_mut()
        {
            if source_id == expected_source {
                *selected_contact = new_selection;
                cx.notify();
            }
        }
    }

    fn open_save_free_chart_modal(
        &mut self,
        source_id: FreeChartId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let default_label = self
            .free_charts
            .iter()
            .find(|e| e.id == source_id)
            .map(|e| e.label.clone())
            .unwrap_or_else(|| "Carta libre".into());
        let all_contacts = self.gather_all_contacts();
        let name_input = self.make_input("Nombre de la carta", &default_label, window, cx);
        let new_contact_input =
            self.make_input("Nombre del contacto nuevo", "", window, cx);
        // Default: primer contacto existente si lo hay; sino "Nuevo
        // contacto" (None). Eso minimiza clicks cuando el usuario ya
        // tiene contactos cargados.
        let selected_contact = all_contacts.first().map(|(id, _)| *id);
        self.modal = Some(Modal::SaveFreeChart {
            source_id,
            name: name_input,
            new_contact_name: new_contact_input,
            selected_contact,
            all_contacts,
            error: None,
        });
        self.close_menu(cx);
    }

    /// Snapshot recursivo de todos los contactos del árbol —
    /// `(id, label)`. Usado por el modal "Guardar como" para
    /// listar destinos. Las cartas se cuelgan del contacto que el
    /// usuario elija.
    fn gather_all_contacts(&self) -> Vec<(ContactId, String)> {
        fn walk(
            store: &Store,
            parent: Option<GroupId>,
            prefix: &str,
            out: &mut Vec<(ContactId, String)>,
        ) {
            if let Ok(contacts) = store.list_contacts(parent) {
                for c in contacts {
                    let label = if prefix.is_empty() {
                        c.name.clone()
                    } else {
                        format!("{}{}", prefix, c.name)
                    };
                    out.push((c.id, label));
                }
            }
            if let Ok(groups) = store.list_groups(parent) {
                for g in groups {
                    let new_prefix = if prefix.is_empty() {
                        format!("{} / ", g.name)
                    } else {
                        format!("{}{} / ", prefix, g.name)
                    };
                    walk(store, Some(g.id), &new_prefix, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.store, None, "", &mut out);
        out
    }

    fn open_create_chart(
        &mut self,
        contact: ContactId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        // Pre-cargamos el nombre del contacto en el campo "Sujeto" del
        // form como conveniencia — la mayoría de las cartas se nombran
        // igual que la persona.
        let subject_name = self
            .store
            .list_contacts(None)
            .ok()
            .and_then(|all| {
                // Buscamos linealmente — list_contacts solo lista hijos
                // directos del group, no nos sirve para encontrar un
                // contact arbitrario. Para fase 2 nos quedamos con
                // "Carta natal" como label genérico si no podemos
                // resolver. Resolver-por-id viene si lo necesitamos.
                let _ = all;
                None::<String>
            })
            .unwrap_or_else(|| "Carta natal".into());

        let form = ChartForm {
            name: self.make_input("Etiqueta de la carta", &subject_name, window, cx),
            place: self.make_input("Lugar (ciudad, país)", "", window, cx),
            year: self.make_input("Año", "1987", window, cx),
            month: self.make_input("Mes", "3", window, cx),
            day: self.make_input("Día", "14", window, cx),
            hour: self.make_input("Hora (0-23)", "5", window, cx),
            minute: self.make_input("Minuto", "22", window, cx),
            tz_offset_min: self.make_input("TZ offset (min)", "-240", window, cx),
            lat: self.make_input("Latitud (°)", "10.4806", window, cx),
            lon: self.make_input("Longitud (°)", "-66.9036", window, cx),
            alt: self.make_input("Altitud (m)", "900", window, cx),
        };
        // El primer field es el que recibe focus.
        form.name.update(cx, |i, _| i.request_focus(window));

        self.modal = Some(Modal::CreateChart {
            contact,
            form,
            error: None,
        });
        self.close_menu(cx);
    }

    fn open_rename(&mut self, target: MenuTarget, window: &mut Window, cx: &mut Context<'_, Self>) {
        let modal = match target {
            MenuTarget::Group(id) => {
                let current = self
                    .store
                    .list_groups(None)
                    .ok()
                    .and_then(|all| find_group_name(&all, &self.store, id))
                    .unwrap_or_default();
                Modal::RenameGroup {
                    id,
                    input: self.make_input("Nuevo nombre", &current, window, cx),
                }
            }
            MenuTarget::Contact(id) => {
                let current = self
                    .store
                    .list_contacts(None)
                    .ok()
                    .and_then(|all| find_contact_name(&all, &self.store, id))
                    .unwrap_or_default();
                Modal::RenameContact {
                    id,
                    input: self.make_input("Nuevo nombre", &current, window, cx),
                }
            }
            MenuTarget::Chart(id) => {
                let current = self
                    .store
                    .get_chart(id)
                    .ok()
                    .map(|c| c.label)
                    .unwrap_or_default();
                Modal::RenameChart {
                    id,
                    input: self.make_input("Nueva etiqueta", &current, window, cx),
                }
            }
            MenuTarget::Root | MenuTarget::FreeChartsRoot | MenuTarget::FreeChart(_) => return,
        };
        self.modal = Some(modal);
        self.close_menu(cx);
    }

    /// Crea un `TextInput` con focus y suscripción a Confirmed/Cancelled.
    /// La closure decide qué hacer con cada evento — guardamos la subscripción
    /// detached para que viva mientras el modal exista.
    fn make_input(
        &self,
        placeholder: &str,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Entity<TextInput> {
        let placeholder = placeholder.to_string();
        let input = cx.new(|cx| {
            TextInput::new(initial.to_string(), cx)
                .with_placeholder(SharedString::from(placeholder.clone()))
        });
        cx.subscribe(&input, |this: &mut Self, _, ev: &TextInputEvent, cx| {
            this.on_input_event(ev, cx);
        })
        .detach();
        input.update(cx, |i, _| i.request_focus(window));
        input
    }

    fn on_input_event(&mut self, ev: &TextInputEvent, cx: &mut Context<'_, Self>) {
        match ev {
            TextInputEvent::Cancelled => self.close_modal(cx),
            TextInputEvent::Confirmed(value) => self.submit_modal(value.clone(), cx),
        }
    }

    fn submit_modal(&mut self, value: String, cx: &mut Context<'_, Self>) {
        let trimmed = value.trim().to_string();
        // Tomamos ownership del modal — si el submit falla en mitad,
        // lo restablecemos. Esto evita un borrow-mut sobre self.modal.
        let modal = match self.modal.take() {
            Some(m) => m,
            None => return,
        };
        match modal {
            Modal::RenameGroup { id, input } => {
                if !trimmed.is_empty() {
                    let _ = self.store.rename_group(id, &trimmed);
                }
                drop(input);
                self.after_mutation(cx);
            }
            Modal::RenameContact { id, input } => {
                if !trimmed.is_empty() {
                    let _ = self.store.rename_contact(id, &trimmed);
                }
                drop(input);
                self.after_mutation(cx);
            }
            Modal::RenameChart { id, input } => {
                if !trimmed.is_empty() {
                    let _ = self.store.rename_chart(id, &trimmed);
                }
                drop(input);
                self.after_mutation(cx);
            }
            Modal::CreateGroup { parent, input } => {
                if !trimmed.is_empty() {
                    let _ = self.store.create_group(parent, &trimmed, None);
                }
                drop(input);
                self.after_mutation(cx);
            }
            Modal::CreateContact { group, input } => {
                if !trimmed.is_empty() {
                    let _ = self.store.create_contact(group, &trimmed, None);
                }
                drop(input);
                self.after_mutation(cx);
            }
            Modal::CreateChart {
                contact,
                form,
                error: _,
            } => {
                // `value` viene del campo que disparó Enter. Para el
                // form, ignoramos el value puntual y leemos todos los
                // campos del form.
                let _ = value;
                match build_chart_from_form(&form, cx) {
                    Ok((birth, label)) => {
                        match self.store.create_chart(
                            contact,
                            ChartKind::Natal,
                            &label,
                            &birth,
                            &StoredChartConfig::default(),
                            None,
                        ) {
                            Ok(_) => {
                                // Auto-expand del contact para que se vea
                                // la carta recién creada.
                                self.expanded
                                    .insert(format!("{}{}", PREFIX_CONTACT, contact));
                                self.after_mutation(cx);
                            }
                            Err(e) => {
                                self.modal = Some(Modal::CreateChart {
                                    contact,
                                    form,
                                    error: Some(SharedString::from(format!("Store: {}", e))),
                                });
                                cx.notify();
                            }
                        }
                    }
                    Err(msg) => {
                        self.modal = Some(Modal::CreateChart {
                            contact,
                            form,
                            error: Some(SharedString::from(msg)),
                        });
                        cx.notify();
                    }
                }
            }
            Modal::EditChart {
                id,
                form,
                error: _,
            } => {
                let _ = value;
                // Para preservar el ChartConfig original (zodiac, house
                // system, bodies, etc.) leemos la carta actual y solo
                // sobrescribimos label + birth_data. El editor no toca
                // config — eso se haría en un futuro panel de "Config
                // de carta".
                let existing = match self.store.get_chart(id) {
                    Ok(c) => c,
                    Err(e) => {
                        self.modal = Some(Modal::EditChart {
                            id,
                            form,
                            error: Some(SharedString::from(format!("Store: {}", e))),
                        });
                        cx.notify();
                        return;
                    }
                };
                match build_chart_from_form(&form, cx) {
                    Ok((birth, label)) => {
                        match self.store.update_chart(id, &label, &birth, &existing.config) {
                            Ok(_) => {
                                drop(form);
                                self.after_mutation(cx);
                            }
                            Err(e) => {
                                self.modal = Some(Modal::EditChart {
                                    id,
                                    form,
                                    error: Some(SharedString::from(format!("Store: {}", e))),
                                });
                                cx.notify();
                            }
                        }
                    }
                    Err(msg) => {
                        self.modal = Some(Modal::EditChart {
                            id,
                            form,
                            error: Some(SharedString::from(msg)),
                        });
                        cx.notify();
                    }
                }
            }
            Modal::EditFreeChart {
                source_id,
                form,
                error: _,
            } => {
                let _ = value;
                match build_chart_from_form(&form, cx) {
                    Ok((birth, label)) => {
                        cx.emit(TreeEvent::FreeChartEditConfirmed {
                            source_id,
                            birth_data: birth,
                            label,
                        });
                        self.modal = None;
                        cx.notify();
                    }
                    Err(msg) => {
                        self.modal = Some(Modal::EditFreeChart {
                            source_id,
                            form,
                            error: Some(SharedString::from(msg)),
                        });
                        cx.notify();
                    }
                }
            }
            Modal::SaveFreeChart {
                source_id,
                name,
                new_contact_name,
                selected_contact,
                all_contacts,
                error: _,
            } => {
                let _ = value;
                let chart_name = name.read(cx).text().to_string();
                let chart_name = chart_name.trim();
                if chart_name.is_empty() {
                    self.modal = Some(Modal::SaveFreeChart {
                        source_id,
                        name,
                        new_contact_name,
                        selected_contact,
                        all_contacts,
                        error: Some("El nombre de la carta no puede estar vacío".into()),
                    });
                    cx.notify();
                    return;
                }
                let new_contact = if selected_contact.is_none() {
                    let v = new_contact_name.read(cx).text().to_string();
                    let v = v.trim();
                    if v.is_empty() {
                        self.modal = Some(Modal::SaveFreeChart {
                            source_id,
                            name,
                            new_contact_name,
                            selected_contact,
                            all_contacts,
                            error: Some(
                                "Elegí un contacto existente o escribí un nombre para el nuevo"
                                    .into(),
                            ),
                        });
                        cx.notify();
                        return;
                    }
                    Some(v.to_string())
                } else {
                    None
                };
                cx.emit(TreeEvent::FreeChartSaveConfirmed {
                    source_id,
                    chart_name: chart_name.to_string(),
                    contact: selected_contact,
                    new_contact_name: new_contact,
                });
                drop(name);
                drop(new_contact_name);
                self.modal = None;
                cx.notify();
            }
        }
    }

    fn after_mutation(&mut self, cx: &mut Context<'_, Self>) {
        self.modal = None;
        self.refresh(cx);
        cx.emit(TreeEvent::HierarchyChanged);
        cx.notify();
    }

    fn confirm_and_delete(
        &mut self,
        target: MenuTarget,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let (label, kind) = match target {
            MenuTarget::Group(_) => ("este grupo (incluye sus subgrupos y contactos)", "group"),
            MenuTarget::Contact(_) => ("este contacto (incluye sus cartas)", "contact"),
            MenuTarget::Chart(_) => ("esta carta", "chart"),
            MenuTarget::Root | MenuTarget::FreeChartsRoot | MenuTarget::FreeChart(_) => return,
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("¿Borrar {}?", label),
            None,
            &["Borrar", "Cancelar"],
            cx,
        );
        let target_clone = target.clone();
        cx.spawn(async move |this, cx| {
            let Ok(idx) = answer.await else { return };
            if idx != 0 {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                match target_clone {
                    MenuTarget::FreeChartsRoot | MenuTarget::FreeChart(_) => {}
                    MenuTarget::Group(id) => {
                        let _ = this.store.delete_group(id);
                    }
                    MenuTarget::Contact(id) => {
                        let _ = this.store.delete_contact(id);
                    }
                    MenuTarget::Chart(id) => {
                        let _ = this.store.delete_chart(id);
                    }
                    MenuTarget::Root => {}
                }
                this.after_mutation(cx);
            });
        })
        .detach();
        let _ = kind;
        self.close_menu(cx);
    }
}

// =====================================================================
// Form helpers
// =====================================================================

fn build_chart_from_form(
    form: &ChartForm,
    cx: &mut Context<'_, TahuantinsuyuTree>,
) -> Result<(StoredBirthData, String), String> {
    let name = form.name.read(cx).text().trim().to_string();
    let place = form.place.read(cx).text().trim().to_string();
    let year: i32 = parse_field(form.year.read(cx).text(), "Año")?;
    let month: u32 = parse_field(form.month.read(cx).text(), "Mes")?;
    let day: u32 = parse_field(form.day.read(cx).text(), "Día")?;
    let hour: u32 = parse_field(form.hour.read(cx).text(), "Hora")?;
    let minute: u32 = parse_field(form.minute.read(cx).text(), "Minuto")?;
    let tz_offset_minutes: i32 = parse_field(form.tz_offset_min.read(cx).text(), "TZ offset")?;
    let latitude_deg: f64 = parse_field(form.lat.read(cx).text(), "Latitud")?;
    let longitude_deg: f64 = parse_field(form.lon.read(cx).text(), "Longitud")?;
    let altitude_m: f64 = parse_field(form.alt.read(cx).text(), "Altitud")?;

    if !(1..=12).contains(&month) {
        return Err(format!("Mes fuera de rango: {}", month));
    }
    if !(1..=31).contains(&day) {
        return Err(format!("Día fuera de rango: {}", day));
    }
    if hour > 23 {
        return Err(format!("Hora fuera de rango: {}", hour));
    }
    if minute > 59 {
        return Err(format!("Minuto fuera de rango: {}", minute));
    }

    let label = if name.is_empty() {
        "Carta natal".to_string()
    } else {
        name
    };
    let birth = StoredBirthData {
        year,
        month,
        day,
        hour,
        minute,
        second: 0.0,
        tz_offset_minutes,
        latitude_deg,
        longitude_deg,
        altitude_m,
        time_certainty: TimeCertainty::Exact,
        subject_name: None,
        birthplace_label: if place.is_empty() { None } else { Some(place) },
    };
    Ok((birth, label))
}

fn parse_field<T: std::str::FromStr>(s: &str, field: &str) -> Result<T, String> {
    s.trim()
        .parse::<T>()
        .map_err(|_| format!("Campo \"{}\" inválido: {:?}", field, s))
}

// =====================================================================
// Lookups auxiliares (DFS por la jerarquía)
// =====================================================================

fn find_group_name(roots: &[cosmos_model::Group], store: &Store, id: GroupId) -> Option<String> {
    for g in roots {
        if g.id == id {
            return Some(g.name.clone());
        }
        if let Ok(children) = store.list_groups(Some(g.id)) {
            if let Some(n) = find_group_name(&children, store, id) {
                return Some(n);
            }
        }
    }
    None
}

fn find_contact_name(
    in_group: &[cosmos_model::Contact],
    store: &Store,
    id: ContactId,
) -> Option<String> {
    for c in in_group {
        if c.id == id {
            return Some(c.name.clone());
        }
    }
    // Buscar también en todos los groups recursivamente.
    if let Ok(groups) = store.list_groups(None) {
        if let Some(n) = find_contact_in_groups(&groups, store, id) {
            return Some(n);
        }
    }
    None
}

fn find_contact_in_groups(
    groups: &[cosmos_model::Group],
    store: &Store,
    id: ContactId,
) -> Option<String> {
    for g in groups {
        if let Ok(cs) = store.list_contacts(Some(g.id)) {
            for c in &cs {
                if c.id == id {
                    return Some(c.name.clone());
                }
            }
        }
        if let Ok(children) = store.list_groups(Some(g.id)) {
            if let Some(n) = find_contact_in_groups(&children, store, id) {
                return Some(n);
            }
        }
    }
    None
}

fn parse_row(id: &RowId) -> Option<TreeSelection> {
    let s = id.as_str();
    if s == ROW_GENERAL {
        return Some(TreeSelection::GeneralRoot);
    }
    if s == ROW_FREE_ROOT {
        return Some(TreeSelection::FreeChartsRoot);
    }
    if let Some(rest) = s.strip_prefix(PREFIX_FREE) {
        return Some(TreeSelection::FreeChart(FreeChartId(rest.to_string())));
    }
    if let Some(rest) = s.strip_prefix(PREFIX_GROUP) {
        return rest.parse().ok().map(TreeSelection::Group);
    }
    if let Some(rest) = s.strip_prefix(PREFIX_CONTACT) {
        return rest.parse().ok().map(TreeSelection::Contact);
    }
    if let Some(rest) = s.strip_prefix(PREFIX_CHART) {
        return rest.parse().ok().map(TreeSelection::Chart);
    }
    None
}

// =====================================================================
// Render
// =====================================================================

const MENU_WIDTH: f32 = 220.0;

impl Render for TahuantinsuyuTree {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let search_bar = div()
            .px(px(6.0))
            .py(px(4.0))
            .border_b_1()
            .border_color(theme.border)
            .child(self.search_input.clone());

        let mut root = div()
            .id("cosmos_app-tree-root")
            .size_full()
            .relative()
            .bg(theme.bg_panel.clone())
            .flex()
            .flex_col()
            .child(search_bar)
            .child(div().flex_grow().min_h(px(0.0)).child(self.inner.clone()));

        if let Some(menu) = self.menu.clone() {
            root = root.child(self.render_menu(&theme, menu, cx));
        }
        if self.modal.is_some() {
            root = root.child(self.render_modal(&theme, cx));
        }
        root
    }
}

impl TahuantinsuyuTree {
    fn render_menu(
        &self,
        theme: &Theme,
        menu: MenuState,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        let mut items = div()
            .flex()
            .flex_col()
            .py(px(4.0))
            .min_w(px(MENU_WIDTH))
            .bg(theme.bg_panel_alt.clone())
            .border_1()
            .border_color(theme.border_strong)
            .rounded(px(6.0));

        match menu.target.clone() {
            MenuTarget::Root => {
                items = items.child(menu_item("tts-menu-new-group", "Nuevo grupo", theme).on_click(
                    cx.listener(|this, _: &ClickEvent, w, cx| {
                        this.open_create_group(None, w, cx);
                    }),
                ));
                items = items.child(
                    menu_item("tts-menu-new-contact-root", "Nuevo contacto", theme).on_click(
                        cx.listener(|this, _: &ClickEvent, w, cx| {
                            this.open_create_contact(None, w, cx);
                        }),
                    ),
                );
            }
            MenuTarget::Group(id) => {
                items = items.child(
                    menu_item("tts-menu-new-subgroup", "Nuevo subgrupo", theme).on_click(
                        cx.listener(move |this, _: &ClickEvent, w, cx| {
                            this.open_create_group(Some(id), w, cx);
                        }),
                    ),
                );
                items = items.child(
                    menu_item("tts-menu-new-contact", "Nuevo contacto", theme).on_click(
                        cx.listener(move |this, _: &ClickEvent, w, cx| {
                            this.open_create_contact(Some(id), w, cx);
                        }),
                    ),
                );
                items = items.child(separator(theme));
                let t = menu.target.clone();
                items = items.child(menu_item("tts-menu-rename-g", "Renombrar…", theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, w, cx| {
                        this.open_rename(t.clone(), w, cx);
                    }),
                ));
                let t = menu.target.clone();
                items = items.child(menu_item("tts-menu-delete-g", "Borrar…", theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, w, cx| {
                        this.confirm_and_delete(t.clone(), w, cx);
                    }),
                ));
            }
            MenuTarget::Contact(id) => {
                items = items.child(menu_item("tts-menu-new-chart", "Nueva carta…", theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, w, cx| {
                        this.open_create_chart(id, w, cx);
                    }),
                ));
                items = items.child(separator(theme));
                let t = menu.target.clone();
                items = items.child(menu_item("tts-menu-rename-c", "Renombrar…", theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, w, cx| {
                        this.open_rename(t.clone(), w, cx);
                    }),
                ));
                let t = menu.target.clone();
                items = items.child(menu_item("tts-menu-delete-c", "Borrar…", theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, w, cx| {
                        this.confirm_and_delete(t.clone(), w, cx);
                    }),
                ));
            }
            MenuTarget::FreeChartsRoot => {
                items = items.child(
                    menu_item("tts-menu-new-free", "Nueva carta libre", theme).on_click(
                        cx.listener(|this, _: &ClickEvent, _w, cx| {
                            cx.emit(TreeEvent::NewFreeChartRequested);
                            this.close_menu(cx);
                        }),
                    ),
                );
            }
            MenuTarget::FreeChart(fid) => {
                let is_sky = fid.is_sky_now();
                let fid_edit = fid.clone();
                items = items.child(
                    menu_item("tts-menu-edit-free", "Editar datos…", theme).on_click(
                        cx.listener(move |this, _: &ClickEvent, w, cx| {
                            this.open_edit_free_chart_modal(fid_edit.clone(), w, cx);
                        }),
                    ),
                );
                let fid_save = fid.clone();
                items = items.child(
                    menu_item("tts-menu-save-free", "Guardar como…", theme).on_click(
                        cx.listener(move |this, _: &ClickEvent, w, cx| {
                            this.open_save_free_chart_modal(fid_save.clone(), w, cx);
                        }),
                    ),
                );
                if !is_sky {
                    items = items.child(separator(theme));
                    let fid_del = fid.clone();
                    items = items.child(
                        menu_item("tts-menu-delete-free", "Borrar", theme).on_click(
                            cx.listener(move |this, _: &ClickEvent, _w, cx| {
                                cx.emit(TreeEvent::DeleteFreeChartRequested(fid_del.clone()));
                                this.close_menu(cx);
                            }),
                        ),
                    );
                }
            }
            MenuTarget::Chart(id) => {
                items = items.child(menu_item("tts-menu-open-h", "Abrir", theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        cx.emit(TreeEvent::Opened(TreeSelection::Chart(id)));
                        this.close_menu(cx);
                    }),
                ));
                items = items.child(menu_item("tts-menu-edit-h", "Editar…", theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, w, cx| {
                        this.open_edit_chart(id, w, cx);
                    }),
                ));
                items = items.child(separator(theme));
                let t = menu.target.clone();
                items = items.child(menu_item("tts-menu-rename-h", "Renombrar…", theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, w, cx| {
                        this.open_rename(t.clone(), w, cx);
                    }),
                ));
                let t = menu.target.clone();
                items = items.child(menu_item("tts-menu-delete-h", "Borrar…", theme).on_click(
                    cx.listener(move |this, _: &ClickEvent, w, cx| {
                        this.confirm_and_delete(t.clone(), w, cx);
                    }),
                ));
            }
        }

        div()
            .absolute()
            .left(menu.position.x)
            .top(menu.position.y)
            .child(items)
    }

    fn render_modal(&self, theme: &Theme, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let modal = self.modal.as_ref().expect("render_modal sin modal activo");
        let inner = match modal {
            Modal::RenameGroup { input, .. }
            | Modal::RenameContact { input, .. }
            | Modal::RenameChart { input, .. } => {
                modal_box(theme, "Renombrar", input.clone(), "Enter = guardar — Escape = cancelar")
            }
            Modal::CreateGroup { input, .. } => {
                modal_box(theme, "Nuevo grupo", input.clone(), "Enter = crear — Escape = cancelar")
            }
            Modal::CreateContact { input, .. } => modal_box(
                theme,
                "Nuevo contacto",
                input.clone(),
                "Enter = crear — Escape = cancelar",
            ),
            Modal::CreateChart { form, error, .. } => render_chart_form(
                theme,
                "Nueva carta natal",
                form,
                error.clone(),
                self.city_picker_open,
                &self.city_atlas,
                cx,
            ),
            Modal::EditChart { form, error, .. } => render_chart_form(
                theme,
                "Editar carta natal",
                form,
                error.clone(),
                self.city_picker_open,
                &self.city_atlas,
                cx,
            ),
            Modal::EditFreeChart { form, error, .. } => render_chart_form(
                theme,
                "Editar carta libre",
                form,
                error.clone(),
                self.city_picker_open,
                &self.city_atlas,
                cx,
            ),
            Modal::SaveFreeChart {
                source_id,
                name,
                new_contact_name,
                selected_contact,
                all_contacts,
                error,
            } => render_save_free_chart(
                theme,
                source_id.clone(),
                name.clone(),
                new_contact_name.clone(),
                *selected_contact,
                all_contacts,
                error.clone(),
                cx,
            ),
        };

        div()
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(0.0, 0.0, 0.0, 0.55))
            .child(inner)
    }
}

// =====================================================================
// Helpers de UI
// =====================================================================

fn menu_item(
    id: &'static str,
    label: &'static str,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px(px(12.0))
        .py(px(6.0))
        .text_size(px(12.0))
        .text_color(theme.fg_text)
        .hover(|s| s.bg(theme.bg_row_hover))
        .child(label)
}

fn separator(theme: &Theme) -> gpui::Div {
    div()
        .my(px(3.0))
        .h(px(1.0))
        .w_full()
        .bg(theme.border)
}

fn modal_box(
    theme: &Theme,
    title: &'static str,
    input: Entity<TextInput>,
    hint: &'static str,
) -> gpui::Div {
    div()
        .min_w(px(380.0))
        .p(px(16.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .bg(theme.bg_panel_alt.clone())
        .border_1()
        .border_color(theme.border_strong)
        .rounded(px(8.0))
        .child(
            div()
                .text_size(px(13.0))
                .text_color(theme.fg_text)
                .child(title),
        )
        .child(input)
        .child(
            div()
                .text_size(px(10.0))
                .text_color(theme.fg_muted)
                .child(hint),
        )
}

/// Modal "Guardar como" para una carta libre. Layout:
///
/// ```text
///   [Nombre de la carta] — TextInput pre-poblado con label
///   Contacto destino:
///     ○ Contacto A
///     ○ Contacto B
///     ● Nuevo contacto…  → [Nombre del contacto] TextInput
///   [Cancelar]  [Guardar]
/// ```
///
/// El submit emite `TreeEvent::FreeChartSaveConfirmed` que el shell
/// materializa contra la store.
#[allow(clippy::too_many_arguments)]
fn render_save_free_chart(
    theme: &Theme,
    source_id: FreeChartId,
    name: Entity<TextInput>,
    new_contact_name: Entity<TextInput>,
    selected_contact: Option<ContactId>,
    all_contacts: &[(ContactId, String)],
    error: Option<SharedString>,
    cx: &mut Context<'_, TahuantinsuyuTree>,
) -> gpui::Div {
    let title_row = div()
        .text_size(px(14.0))
        .text_color(theme.fg_text)
        .child(SharedString::from("Guardar carta libre"));
    let label_row =
        |label: &'static str| -> gpui::Div {
            div()
                .text_size(px(10.0))
                .text_color(theme.fg_muted)
                .child(label)
        };
    let name_block = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(label_row("Nombre"))
        .child(name.clone());

    // Lista de contactos como botones radio.
    let mut contact_list = div().flex().flex_col().gap(px(4.0));
    for (cid, label) in all_contacts.iter() {
        let selected = selected_contact == Some(*cid);
        let cid_for_click = *cid;
        let source_for_click = source_id.clone();
        let row_id: SharedString =
            SharedString::from(format!("tts-save-pick-{}", cid));
        let bullet = if selected { "●" } else { "○" };
        let row = div()
            .id(gpui::ElementId::from(row_id))
            .flex()
            .flex_row()
            .gap(px(8.0))
            .px(px(6.0))
            .py(px(3.0))
            .rounded(px(4.0))
            .text_size(px(11.0))
            .text_color(theme.fg_text)
            .hover(|s| s.bg(theme.bg_row_hover))
            .child(bullet.to_string())
            .child(label.clone())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.set_save_modal_contact(Some(cid_for_click), &source_for_click, cx);
            }));
        contact_list = contact_list.child(row);
    }
    // Opción "Nuevo contacto…" — bullet activo si selected_contact==None.
    let new_selected = selected_contact.is_none();
    let new_bullet = if new_selected { "●" } else { "○" };
    let source_for_new = source_id.clone();
    contact_list = contact_list.child(
        div()
            .id(gpui::ElementId::from(SharedString::from(
                "tts-save-pick-new",
            )))
            .flex()
            .flex_row()
            .gap(px(8.0))
            .px(px(6.0))
            .py(px(3.0))
            .rounded(px(4.0))
            .text_size(px(11.0))
            .text_color(theme.fg_text)
            .hover(|s| s.bg(theme.bg_row_hover))
            .child(new_bullet.to_string())
            .child("Nuevo contacto…")
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.set_save_modal_contact(None, &source_for_new, cx);
            })),
    );

    let mut contacts_block = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(label_row("Contacto destino"))
        .child(contact_list);

    // Si "Nuevo contacto" está activo, mostrar el TextInput debajo.
    if new_selected {
        contacts_block = contacts_block.child(
            div()
                .pt(px(4.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(label_row("Nombre del contacto nuevo"))
                .child(new_contact_name.clone()),
        );
    }

    let save_btn = div()
        .id("tts-save-free-confirm")
        .px(px(14.0))
        .py(px(8.0))
        .rounded(px(6.0))
        .bg(theme.bg_button())
        .hover(|s| s.bg(theme.bg_button_hover()))
        .text_size(px(12.0))
        .text_color(theme.fg_text)
        .child("Guardar")
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.submit_modal(String::new(), cx);
        }));
    let cancel_btn = div()
        .id("tts-save-free-cancel")
        .px(px(14.0))
        .py(px(8.0))
        .rounded(px(6.0))
        .bg(theme.bg_panel.clone())
        .hover(|s| s.bg(theme.bg_row_hover))
        .text_size(px(12.0))
        .text_color(theme.fg_muted)
        .child("Cancelar")
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.close_modal(cx);
        }));

    let mut body = div()
        .min_w(px(420.0))
        .p(px(18.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .bg(theme.bg_panel_alt.clone())
        .border_1()
        .border_color(theme.border_strong)
        .rounded(px(8.0))
        .child(title_row)
        .child(name_block)
        .child(contacts_block);
    if let Some(err) = error {
        body = body.child(
            div()
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(4.0))
                .bg(theme.bg_destructive_hover())
                .text_size(px(11.0))
                .text_color(theme.accent_destructive())
                .child(err),
        );
    }
    body = body.child(
        div()
            .flex()
            .flex_row()
            .gap(px(8.0))
            .justify_end()
            .child(cancel_btn)
            .child(save_btn),
    );
    body
}

fn render_chart_form(
    theme: &Theme,
    title: &str,
    form: &ChartForm,
    error: Option<SharedString>,
    // Datos del tree que el form necesita renderizar — recibidos por
    // parámetro porque esta función se llama desde `render()` y la
    // entity del tree ya está leased; un `cx.entity().read(cx)`
    // adentro causa double_lease_panic en gpui.
    picker_open: bool,
    city_atlas: &[CityPreset],
    cx: &mut Context<'_, TahuantinsuyuTree>,
) -> gpui::Div {
    let labeled = |label: &'static str, input: Entity<TextInput>| -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(theme.fg_muted)
                    .child(label),
            )
            .child(input)
    };

    let date_row = div()
        .flex()
        .flex_row()
        .gap(px(8.0))
        .child(labeled("Año", form.year.clone()))
        .child(labeled("Mes", form.month.clone()))
        .child(labeled("Día", form.day.clone()))
        .child(labeled("Hora", form.hour.clone()))
        .child(labeled("Minuto", form.minute.clone()))
        .child(labeled("TZ (min)", form.tz_offset_min.clone()));

    let loc_row = div()
        .flex()
        .flex_row()
        .gap(px(8.0))
        .child(labeled("Latitud", form.lat.clone()))
        .child(labeled("Longitud", form.lon.clone()))
        .child(labeled("Altitud (m)", form.alt.clone()));

    let create_btn = div()
        .id("tts-chart-form-create")
        .px(px(14.0))
        .py(px(8.0))
        .rounded(px(6.0))
        .bg(theme.bg_button())
        .hover(|s| s.bg(theme.bg_button_hover()))
        .text_size(px(12.0))
        .text_color(theme.fg_text)
        .child(SharedString::from(if title.starts_with("Editar") {
            "Guardar cambios"
        } else {
            "Crear carta"
        }))
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            // Disparamos un submit "vacío" — el handler de submit
            // re-lee todos los campos del form. El value que pasamos
            // se ignora dentro del branch CreateChart/EditChart.
            this.submit_modal(String::new(), cx);
        }));

    let cancel_btn = div()
        .id("tts-chart-form-cancel")
        .px(px(14.0))
        .py(px(8.0))
        .rounded(px(6.0))
        .bg(theme.bg_panel.clone())
        .hover(|s| s.bg(theme.bg_row_hover))
        .text_size(px(12.0))
        .text_color(theme.fg_muted)
        .child("Cancelar")
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.close_modal(cx);
        }));

    // Header del form: title + botón "Ciudad rápida" con dropdown
    // que autocompleta place/lat/lon/tz al elegir un preset.
    let city_btn = div()
        .id("tts-form-city-btn")
        .px(px(10.0))
        .py(px(4.0))
        .rounded(px(4.0))
        .bg(theme.bg_button())
        .hover(|s| s.bg(theme.bg_button_hover()))
        .border_1()
        .border_color(if picker_open {
            theme.accent_strong
        } else {
            theme.border
        })
        .text_size(px(11.0))
        .text_color(theme.fg_text)
        .child("📍 Ciudad rápida ▾")
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.toggle_city_picker(cx);
        }));
    let title_row = div()
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .child(
            div()
                .text_size(px(14.0))
                .text_color(theme.fg_text)
                .child(SharedString::from(title.to_string())),
        )
        .child(div().flex_grow())
        .child(city_btn);
    let title_row = if picker_open {
        let popup_id: SharedString = SharedString::from("tts-form-city-popup");
        let mut popup = div()
            .id(gpui::ElementId::from(popup_id))
            .absolute()
            .top(px(36.0))
            .right(px(0.0))
            .min_w(px(240.0))
            .h(px(360.0))
            .py(px(4.0))
            .bg(theme.bg_panel_alt.clone())
            .border_1()
            .border_color(theme.border_strong)
            .rounded(px(6.0))
            .flex()
            .flex_col()
            .overflow_y_scroll();
        for preset in city_atlas.iter().cloned() {
            let row_id: SharedString =
                SharedString::from(format!("tts-city-{}", preset.name));
            let label = preset.name.clone();
            popup = popup.child(
                div()
                    .id(gpui::ElementId::from(row_id))
                    .px(px(10.0))
                    .py(px(4.0))
                    .text_size(px(11.0))
                    .text_color(theme.fg_text)
                    .hover(|s| s.bg(theme.bg_row_hover))
                    .child(SharedString::from(label))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.apply_city_preset(&preset, cx);
                    })),
            );
        }
        title_row.child(popup)
    } else {
        title_row
    };

    let mut body = div()
        .min_w(px(640.0))
        .p(px(18.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .bg(theme.bg_panel_alt.clone())
        .border_1()
        .border_color(theme.border_strong)
        .rounded(px(8.0))
        .child(title_row)
        .child(
            div()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .child(labeled("Etiqueta", form.name.clone()))
                .child(labeled("Lugar (texto libre)", form.place.clone())),
        )
        .child(date_row)
        .child(loc_row);

    if let Some(err) = error {
        body = body.child(
            div()
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(4.0))
                .bg(theme.bg_destructive_hover())
                .text_size(px(11.0))
                .text_color(theme.accent_destructive())
                .child(err),
        );
    }

    body = body.child(
        div()
            .flex()
            .flex_row()
            .gap(px(8.0))
            .justify_end()
            .child(cancel_btn)
            .child(create_btn),
    );

    body
}
