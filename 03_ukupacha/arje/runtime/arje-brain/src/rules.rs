//! Tipos de regla. La fuente de verdad del shape es esta definición Rust;
//! `schema/rule.k` queda como referencia de diseño no cargada.
//!
//! Cargables desde JSON (array, objeto-con-array, o JSONL). El motor no
//! acepta Rules construidas a mano sin pasar por validate() — ver
//! `engine::insert`.

use arje_card::Capability;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Triplet [Sujeto + Evento + Acción]. Inmutable tras carga.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: Ulid,
    #[serde(default = "default_priority")]
    pub priority: u8,
    pub when: EventPattern,
    pub then: Vec<Action>,
    #[serde(default)]
    pub scope: Scope,
}

fn default_priority() -> u8 { 5 }

impl Rule {
    pub fn validate(&self) -> Result<(), RuleError> {
        if self.then.is_empty() {
            return Err(RuleError::EmptyActions);
        }
        self.when.validate_recursive()
    }
}

#[derive(Debug)]
pub enum RuleError {
    EmptyActions,
    EmptySequence,
    EmptyCompound,
    InvalidPriority,
}

impl std::fmt::Display for RuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyActions => write!(f, "regla sin acciones"),
            Self::EmptySequence => write!(f, "Sequence pattern con kinds vacío"),
            Self::EmptyCompound => write!(f, "Either/All con patterns vacío"),
            Self::InvalidPriority => write!(f, "prioridad fuera de rango"),
        }
    }
}

impl std::error::Error for RuleError {}

/// Match del sujeto. Vacío en todos los campos = match cualquier Ente.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<Ulid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_has_cap: Option<Capability>,
}

impl Scope {
    pub fn is_wildcard(&self) -> bool {
        self.subject_id.is_none()
            && self.subject_label.is_none()
            && self.subject_has_cap.is_none()
    }
}

/// Patrón de evento que dispara una regla. Tagged union — JSON con campo
/// `type`. Soporta composición recursiva (Either/All) sobre Single y
/// Sequence atómicos.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "type")]
pub enum EventPattern {
    /// Match un único evento por kind.
    Single { kind: EventKind },
    /// Match si los últimos N eventos del history coinciden en orden con
    /// `kinds`, todos dentro de `within_ms` (0 = sin límite temporal).
    Sequence {
        kinds: Vec<EventKind>,
        #[serde(default)]
        within_ms: u64,
    },
    /// OR: match si cualquier sub-pattern matchea.
    Either { patterns: Vec<EventPattern> },
    /// AND: match si todos los sub-patterns matchean simultáneamente
    /// contra el mismo (event, history).
    All { patterns: Vec<EventPattern> },
}

impl EventPattern {
    /// `true` si el pattern es atómico (no recursivo) y puede ser indexado
    /// por discriminante de `EventKind` para dispatch O(1). Compuestos
    /// (Either/All) se evalúan en un bucket de fallback.
    pub fn is_simple(&self) -> bool {
        matches!(self, Self::Single { .. } | Self::Sequence { .. })
    }

    /// Última `EventKind` que dispara la evaluación de un pattern atómico.
    /// Devuelve None para compuestos.
    pub fn trigger_kind(&self) -> Option<&EventKind> {
        match self {
            Self::Single { kind } => Some(kind),
            Self::Sequence { kinds, .. } => kinds.last(),
            Self::Either { .. } | Self::All { .. } => None,
        }
    }

    /// Validación recursiva. Compuestos vacíos se rechazan al cargar.
    pub fn validate_recursive(&self) -> Result<(), RuleError> {
        match self {
            Self::Single { .. } => Ok(()),
            Self::Sequence { kinds, .. } => {
                if kinds.is_empty() { Err(RuleError::EmptySequence) } else { Ok(()) }
            }
            Self::Either { patterns } | Self::All { patterns } => {
                if patterns.is_empty() {
                    return Err(RuleError::EmptyCompound);
                }
                for p in patterns { p.validate_recursive()?; }
                Ok(())
            }
        }
    }
}

/// Tipo de evento que dispara reglas. Indexado por discriminante en el motor.
/// Diseñado para que `EventKindDiscriminant::from(&kind)` sea barato (no hash
/// del payload, sólo del tag).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EventKind {
    EnteSpawned,
    EnteDied,
    BusAnnounce,
    BusInvoke,
    BusInvokeOf(Capability),
    DeviceAdded,
    DeviceRemoved,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel { Trace, Debug, Info, Warn, Error }

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase")]
pub enum Action {
    Log {
        #[serde(default = "default_log_level")]
        level: LogLevel,
        message: String,
    },
    Notify {
        target_id: Ulid,
        message: String,
    },
    /// `card_blob` es JSON del EntityCard codificado en base64. El motor lo
    /// decodifica y forwarda como SpawnRequest al graph.
    Spawn {
        card_blob: String,
    },
    Invoke {
        target_cap: Capability,
        /// blob crudo (en JSON viaja como base64 vía `blob_b64`).
        #[serde(with = "blob_b64")]
        blob: Vec<u8>,
    },
    Inhibit {
        reason: String,
    },
}

fn default_log_level() -> LogLevel { LogLevel::Info }

mod blob_b64 {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        STANDARD.decode(&s).map_err(serde::de::Error::custom)
    }
}
