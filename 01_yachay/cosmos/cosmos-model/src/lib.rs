//! `cosmos_app-model` — tipos agnósticos del estudio astrológico.
//!
//! Esta es la capa de **datos puros**: no conoce GPUI, ni rusqlite, ni
//! `eternal-astrology`. Solo tipos `serde`-able que viajan entre la
//! store, la engine, los widgets, y eventualmente la Card de Brahman.
//!
//! ## Jerarquía
//!
//! ```text
//!   Group  (puede anidar otros Groups vía parent_id)
//!     ├── Group  (sub-agrupación)
//!     └── Contact  (persona / evento / lugar)
//!           └── Chart  (carta astrológica)
//! ```
//!
//! Las `Chart` son las hojas — cada una guarda su `StoredBirthData` y su
//! `StoredChartConfig`. La engine las traduce a tipos de `eternal-astrology`
//! cuando hay que computar.
//!
//! ## Por qué tipos "Stored" propios y no reusar `eternal-astrology`
//!
//! Forward-compat: si mañana cambia el shape de `BirthData` upstream, o
//! queremos persistir en otro backend astronómico, el modelo + la base
//! sobreviven. La engine es el único puente que conoce ambas formas.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ulid::Ulid;

pub use ::ulid;

// =====================================================================
// Identidades
// =====================================================================

macro_rules! ulid_newtype {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub Ulid);

        impl $name {
            pub fn new() -> Self {
                Self(Ulid::new())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = ulid::DecodeError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ulid::from_string(s).map(Self)
            }
        }
    };
}

ulid_newtype!(GroupId, "Identificador estable de un Group.");
ulid_newtype!(ContactId, "Identificador estable de un Contact.");
ulid_newtype!(ChartId, "Identificador estable de un Chart.");

// =====================================================================
// Group / Contact
// =====================================================================

/// Agrupación jerárquica de contactos. Puede anidar otros groups vía
/// `parent_id` (un Group raíz tiene `parent_id = None`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub parent_id: Option<GroupId>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Epoch millis. Decisión: `i64` para tolerar valores pre-1970 en
    /// imports históricos sin overflow.
    pub created_at_ms: i64,
    /// Orden manual dentro del padre. Más bajo = primero. Empate → por nombre.
    #[serde(default)]
    pub sort_order: i32,
}

/// Persona o evento del que se calcula una o más cartas. Puede vivir
/// directamente en la raíz (`group_id = None`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub id: ContactId,
    pub group_id: Option<GroupId>,
    pub name: String,
    #[serde(default)]
    pub notes: Option<String>,
    pub created_at_ms: i64,
}

// =====================================================================
// Datos de nacimiento (espejo agnóstico de cosmos_astrology::BirthData)
// =====================================================================

/// Datos crudos de nacimiento. La engine los traduce a
/// `cosmos_astrology::BirthData` cuando hay que computar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredBirthData {
    /// Calendario civil local.
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    /// Segundos fraccionarios (0.0..60.0).
    pub second: f64,
    /// Offset desde UTC, en minutos. Ej: -240 = UTC-04:00.
    pub tz_offset_minutes: i32,

    /// Coordenadas geográficas en grados decimales.
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    /// Altura en metros sobre el geoide WGS-84.
    #[serde(default)]
    pub altitude_m: f64,

    #[serde(default)]
    pub time_certainty: TimeCertainty,
    #[serde(default)]
    pub subject_name: Option<String>,
    #[serde(default)]
    pub birthplace_label: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeCertainty {
    #[default]
    Exact,
    RoundedHour,
    RoundedDay,
    Estimated,
}

// =====================================================================
// Configuración de carta (espejo agnóstico de cosmos_astrology::ChartConfig)
// =====================================================================

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Zodiac {
    #[default]
    Tropical,
    Sidereal,
    Draconic,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HouseSystem {
    #[default]
    Placidus,
    Koch,
    Regiomontanus,
    Campanus,
    Porphyry,
    Equal,
    WholeSign,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredChartConfig {
    #[serde(default)]
    pub zodiac: Zodiac,
    #[serde(default)]
    pub house_system: HouseSystem,
    /// Nombre del ayanamsha cuando `zodiac == Sidereal`. Ej: "lahiri",
    /// "fagan_bradley". Ignorado para Tropical/Draconic.
    #[serde(default)]
    pub ayanamsha: Option<String>,
    /// Cuerpos a incluir. Strings opacos para que el modelo no se ate
    /// al enum `Body` de eternal. Ej: ["sun","moon","mercury",…].
    #[serde(default = "default_bodies")]
    pub bodies: Vec<String>,
    #[serde(default = "default_true")]
    pub include_south_node: bool,
    #[serde(default)]
    pub include_lilith: bool,
    #[serde(default)]
    pub include_main_belt_asteroids: bool,
    #[serde(default)]
    pub include_fixed_stars: bool,
    /// Tabla de orbes a usar (nombre simbólico). `None` → orbes defaults
    /// de la engine.
    #[serde(default)]
    pub orb_table: Option<String>,
}

impl Default for StoredChartConfig {
    fn default() -> Self {
        Self {
            zodiac: Zodiac::default(),
            house_system: HouseSystem::default(),
            ayanamsha: None,
            bodies: default_bodies(),
            include_south_node: true,
            include_lilith: false,
            include_main_belt_asteroids: false,
            include_fixed_stars: false,
            orb_table: None,
        }
    }
}

fn default_bodies() -> Vec<String> {
    vec![
        "sun", "moon", "mercury", "venus", "mars", "jupiter", "saturn", "uranus", "neptune",
        "pluto", "mean_node",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn default_true() -> bool {
    true
}

// =====================================================================
// Chart
// =====================================================================

/// Tipo de carta astrológica. Determina qué rutina de la engine corre
/// y qué `Layer`s aporta al canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    Natal,
    Transit,
    SecondaryProgression,
    TertiaryProgression,
    MinorProgression,
    SolarArc,
    SolarReturn,
    LunarReturn,
    Synastry,
    Composite,
    Davison,
    Profection,
    PrimaryDirection,
    /// Carta "mundial" para un instante + lugar sin sujeto natal.
    Mundane,
}

impl ChartKind {
    /// `true` si la carta necesita una segunda carta natal como referencia
    /// (synastry/composite/davison). Útil para validar al persistir.
    pub fn requires_related_chart(self) -> bool {
        matches!(
            self,
            ChartKind::Synastry | ChartKind::Composite | ChartKind::Davison
        )
    }
}

/// Una carta concreta dentro de un contacto. Las cartas de tipo
/// derivado (transit, progression, synastry, …) referencian la carta
/// natal de la que parten vía `related_chart_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chart {
    pub id: ChartId,
    pub contact_id: ContactId,
    pub kind: ChartKind,
    pub label: String,
    pub birth_data: StoredBirthData,
    pub config: StoredChartConfig,
    /// Para cartas derivadas: la carta de referencia. Para transit/
    /// progression apunta a la natal del mismo contacto. Para synastry
    /// apunta a la carta del otro sujeto.
    #[serde(default)]
    pub related_chart_id: Option<ChartId>,
    pub created_at_ms: i64,
}

// =====================================================================
// Estado de módulos por carta (qué capas están activas + su config)
// =====================================================================

/// Cada `ChartKind` puede activar uno o más `module_id` (ej. una carta
/// natal puede tener `natal`, `dignities`, `fixed_stars`, `uranian`).
/// El estado por-carta se persiste en la store; el canvas lo consulta
/// para decidir qué capas pintar y qué controles mostrar en el panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleState {
    pub chart_id: ChartId,
    pub module_id: String,
    pub enabled: bool,
    /// JSON libre — cada módulo define su schema.
    #[serde(default)]
    pub config: serde_json::Value,
}

// =====================================================================
// Selección activa (qué muestra el canvas)
// =====================================================================

/// Identificador de una carta "libre" — efímera, no persistida en la
/// store. Llave de un `HashMap` en el shell. El valor `SKY_NOW_ID`
/// está reservado para la carta del instante actual; otros se
/// generan al vuelo como UUIDs string-encoded.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FreeChartId(pub String);

impl FreeChartId {
    pub fn sky_now() -> Self {
        Self(SKY_NOW_ID.into())
    }
    pub fn is_sky_now(&self) -> bool {
        self.0 == SKY_NOW_ID
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Sentinela del id de la carta "Cielo ahora" — siempre presente
/// como primer elemento de la sección "Cartas libres" del tree.
pub const SKY_NOW_ID: &str = "sky-now";

/// Item activo del tree. El canvas reacciona a este tipo:
/// - `Chart` → abre la carta puntual.
/// - `Contact` / `Group` → muestra thumbnails de las cartas descendientes.
/// - `FreeChart` → carta libre (no anclada a contacto). Incluye la
///   especial "Cielo ahora" + cualquier creada por el usuario.
/// - `FreeChartsRoot` → branch virtual de la sección "Cartas libres".
/// - `GeneralRoot` → nodo branch virtual que agrupa los contactos
///   sin grupo padre (contacts con parent=None).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TreeSelection {
    Group(GroupId),
    Contact(ContactId),
    Chart(ChartId),
    FreeChart(FreeChartId),
    FreeChartsRoot,
    GeneralRoot,
}

// =====================================================================
// Errores
// =====================================================================

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("chart {kind:?} requiere related_chart_id pero recibió None")]
    MissingRelatedChart { kind: ChartKind },
    #[error("group {0} no puede ser su propio ancestro")]
    GroupCycle(GroupId),
    #[error("invalid field {field}: {reason}")]
    InvalidField {
        field: &'static str,
        reason: String,
    },
}

impl Chart {
    /// Validación liviana: ataja errores que la base no captura
    /// (ej. synastry sin `related_chart_id`).
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.kind.requires_related_chart() && self.related_chart_id.is_none() {
            return Err(ModelError::MissingRelatedChart { kind: self.kind });
        }
        if !(-90.0..=90.0).contains(&self.birth_data.latitude_deg) {
            return Err(ModelError::InvalidField {
                field: "latitude_deg",
                reason: format!("{} fuera de [-90, 90]", self.birth_data.latitude_deg),
            });
        }
        if !(-180.0..=180.0).contains(&self.birth_data.longitude_deg) {
            return Err(ModelError::InvalidField {
                field: "longitude_deg",
                reason: format!("{} fuera de [-180, 180]", self.birth_data.longitude_deg),
            });
        }
        Ok(())
    }
}
