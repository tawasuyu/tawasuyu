//! `brahman-card` — Tarjeta de Presentación canónica de Brahman.
//!
//! Híbrida del `EntityCard` de arje (identidad ULID, capacidades tipadas,
//! `Payload`/`SomaSpec`/`Supervision`/`genesis` recursivo) con flujos tipados,
//! permisos enumerados explícitos y nivel de confianza derivado del modelo
//! que veníamos diseñando en `core_protocol`. Una sola tarjeta sirve a:
//!
//! - **El Init** (encarnación): `payload` + `soma` + `supervision` + `genesis`.
//! - **El Admin** (matching): `provides`/`requires` + `flow` + `permissions`.
//! - **El runtime** (sandbox): `permissions` enumerados → seccomp / namespaces.
//!
//! Forward-compat: cualquier campo desconocido se preserva en `extensions`
//! (raíz) o en `extra` (sub-secciones).
//!
//! Formatos soportados: JSON (canónico, compatible con arje) y TOML
//! (humano-legible). Auto-detección por extensión.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use thiserror::Error;
use ulid::Ulid;

// Re-export para que los consumidores no necesiten depender de `ulid`
// directamente.
pub use ::ulid;

/// Versión del esquema de la Card.
pub const CARD_SCHEMA_VERSION: u16 = 1;

/// Versión del protocolo Brahman.
pub const PROTOCOL_VERSION: &str = "0.1.0";

/// Errores de parseo o validación de la Card.
#[derive(Debug, Error)]
pub enum CardError {
    #[error("schema version mismatch: got {got}, expected {expected}")]
    SchemaMismatch { got: u16, expected: u16 },
    #[error("label vacío")]
    EmptyLabel,
    #[error("label demasiado largo: {0} bytes (máx 256)")]
    LabelTooLong(usize),
    #[error("capacidad presente en provides Y requires: {0:?}")]
    SelfDependency(Capability),
    #[error("payload Native/Legacy con exec vacío")]
    EmptyExec,
    #[error("payload Wasm con sha256 sentinela (todo ceros)")]
    SentinelWasmHash,
    #[error("rlimit inválido: {0}")]
    InvalidRlimit(&'static str),
    #[error("cgroup weight fuera de [1,10000]: {0}")]
    InvalidCgroupWeight(&'static str),
    #[error("flujo {section}: nombre duplicado '{name}'")]
    DuplicateFlowName {
        section: &'static str,
        name: String,
    },
    #[error("JSON inválido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("TOML inválido: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("E/S leyendo card: {0}")]
    Io(#[from] std::io::Error),
    #[error("formato desconocido (extensiones esperadas: .json, .toml)")]
    UnknownFormat,
}

// =====================================================================
// Card raíz
// =====================================================================

/// Tarjeta de Presentación de un módulo Brahman.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    /// Versión del esquema. Cambiar = romper compatibilidad del fractal.
    pub schema_version: u16,

    /// Identidad opaca, única en el grafo del fractal.
    pub id: Ulid,

    /// Ancestro del que esta Card desciende (genealogía).
    #[serde(default)]
    pub lineage: Option<Ulid>,

    /// Nombre humano-legible. Único por convención, no por validación.
    pub label: String,

    /// Capacidades del sistema que esta Card ofrece a otros.
    #[serde(default)]
    pub provides: BTreeSet<Capability>,

    /// Capacidades que necesita resolver el Init antes de encarnarla.
    #[serde(default)]
    pub requires: BTreeSet<Capability>,

    /// Permisos sandbox declarativos (más alto nivel que `Capability`).
    /// El Admin los compila a seccomp/namespaces/cgroups concretos.
    #[serde(default)]
    pub permissions: Permissions,

    /// Spec runtime Linux (namespaces, cgroups, rlimits, cpu_affinity).
    #[serde(default)]
    pub soma: SomaSpec,

    /// Qué encarnar: WASM, ELF nativo, virtual, o legacy con shims.
    pub payload: Payload,

    /// Política de supervisión (restart con backoff, oneshot, delegada).
    pub supervision: Supervision,

    /// Modelo de ejecución (eje ortogonal a `supervision`).
    #[serde(default)]
    pub lifecycle: Lifecycle,

    /// Prioridad de scheduling.
    #[serde(default)]
    pub priority: Priority,

    /// Contratos de flujo de datos: qué consume, qué produce.
    #[serde(default)]
    pub flow: Flows,

    /// Si la entidad expone un socket Unix de servicio (data plane,
    /// distinto al socket del Init), declara aquí su path. Los
    /// consumidores que reciban un `MatchEvent` con este Card como
    /// productor pueden conectar directo al socket sin discovery
    /// adicional.
    #[serde(default)]
    pub service_socket: Option<PathBuf>,

    /// Referencias a otras Cards: "soy procesado por X", "poseo Y",
    /// etc. Forma el grafo de relaciones del fractal. Cada Card las
    /// declara unilateralmente; los consumidores pueden cruzarlas para
    /// reconstruir vínculos bidireccionales.
    #[serde(default)]
    pub references: Vec<CardReference>,

    /// Naturaleza de la entidad detrás de la Card. Por defecto `Ente`
    /// para mantener compatibilidad con Cards existentes.
    #[serde(default)]
    pub kind: CardKind,

    /// Faceta de datos cuando `kind != Ente`. `None` para entes
    /// runtime; `Some(...)` para Mónadas, índices, etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<DataFacet>,

    /// Hijas a instanciar inmediatamente al encarnar esta Card.
    #[serde(default)]
    pub genesis: Vec<Card>,

    /// Biases per-contexto. La key es el nombre del contexto (p. ej.
    /// `"test"`, `"prod"`, `"foreground"`). Cuando el broker está
    /// configurado bajo ese contexto, el bias se aplica. Sin contexto
    /// activo o sin entrada matching, este campo no afecta el ranking.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub priority_contexts: BTreeMap<String, ContextBias>,

    /// Campos JSON/TOML desconocidos preservados durante I/O de archivos
    /// (forward-compat). **No se transmiten por wire (postcard)** — la
    /// proyección a [`WireCard`] los descarta porque `serde_json::Value`
    /// no es postcard-friendly. Sirven para anotaciones locales que
    /// sobreviven leer/escribir Cards en disco.
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, JsonValue>,
}

impl Default for Card {
    /// Default determinista pensado para el patrón `..Default::default()`
    /// en struct-literals donde el caller sobreescribe `id` y `label`.
    ///
    /// **Trap conocida**: `id` queda en `Ulid::nil()`. Si construís una
    /// Card "viva" para registrar en el broker, NUNCA dejes el `id`
    /// derivado de `Default` — todas las Cards default-construidas
    /// colisionarían bajo el mismo `00000000000000000000000000`. Para
    /// Cards frescas usá [`Card::new`], que asigna `Ulid::new()`.
    /// `Ulid::nil()` queda reservado para patterns de búsqueda y
    /// sentinel values en serialización.
    fn default() -> Self {
        Self {
            schema_version: CARD_SCHEMA_VERSION,
            id: Ulid::nil(),
            lineage: None,
            label: String::new(),
            provides: BTreeSet::new(),
            requires: BTreeSet::new(),
            permissions: Permissions::default(),
            soma: SomaSpec::default(),
            payload: Payload::Virtual,
            supervision: Supervision::OneShot,
            lifecycle: Lifecycle::default(),
            priority: Priority::default(),
            flow: Flows::default(),
            genesis: Vec::new(),
            service_socket: None,
            references: Vec::new(),
            kind: CardKind::default(),
            data: None,
            priority_contexts: BTreeMap::new(),
            extensions: BTreeMap::new(),
        }
    }
}

// =====================================================================
// Capacidades — heredadas de arje, tipadas, no strings
// =====================================================================

/// Capacidad del sistema. Identificadores tipados, no strings libres.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Provee un punto de montaje root para Cards hijas.
    FilesystemRoot,
    /// Acceso a una familia netlink del kernel.
    KernelNetlink(NetlinkFamily),
    /// Endpoint del bus interno — equivalente tipado de un nombre D-Bus.
    Endpoint {
        interface: InterfaceId,
        version: u16,
    },
    /// Reemplazo del shim de systemd-logind. Solo el ente compat lo provee.
    LegacyLogind,
    /// Acceso crudo a una clase de dispositivo. Capacidad escalada.
    Device { class: DeviceClass },
    /// Permiso de instanciar Cards hijas. Por defecto solo PID 1 lo tiene.
    Spawn,
    /// Acceso a logging estructurado del fractal.
    Journal,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NetlinkFamily {
    Uevent,
    Route,
    Generic,
    Audit,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DeviceClass {
    Block,
    Tty,
    Input,
    Drm,
    Net,
    Hidraw,
}

/// Identificador de interfaz del bus interno (UUID, no string libre).
/// Para extender el protocolo, se genera un UUID nuevo y se versiona.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InterfaceId(pub [u8; 16]);

// =====================================================================
// Permisos sandbox — más alto nivel que Capability
// =====================================================================

/// Permisos declarativos. El Admin los traduce a seccomp/namespaces.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Permissions {
    #[serde(default)]
    pub networking: NetworkingPolicy,
    #[serde(default)]
    pub filesystem: FsPolicy,
    #[serde(default)]
    pub ipc: IpcPolicy,
    /// Capacidad de spawnear sub-procesos. Implica `TrustLevel::System`.
    #[serde(default)]
    pub processes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkingPolicy {
    #[default]
    None,
    Loopback,
    Outbound,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FsPolicy {
    #[default]
    None,
    ReadOnly,
    ReadWrite,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IpcPolicy {
    /// Protocolos IPC permitidos (p. ej. `"wit-v1"`, `"shm-v1"`).
    #[serde(default)]
    pub allow: Vec<String>,
}

// =====================================================================
// SomaSpec — runtime Linux (heredado de arje sin cambios)
// =====================================================================

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SomaSpec {
    pub namespaces: NamespaceSet,
    pub rlimits: ResourceLimits,
    pub cgroup: CgroupSpec,
    pub cpu_affinity: Option<Vec<u32>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NamespaceSet {
    pub mount: bool,
    pub pid: bool,
    pub net: bool,
    pub uts: bool,
    pub ipc: bool,
    pub user: bool,
    pub cgroup: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub mem_bytes: Option<u64>,
    pub nproc: Option<u32>,
    pub nofile: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CgroupSpec {
    #[serde(default)]
    pub path: String,
    pub cpu_weight: Option<u32>,
    pub io_weight: Option<u32>,
}

// =====================================================================
// Payload — qué encarnar (heredado de arje)
// =====================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Payload {
    Wasm {
        module_sha256: [u8; 32],
        entry: String,
    },
    Native {
        exec: String,
        argv: Vec<String>,
        envp: Vec<(String, String)>,
    },
    /// Sin proceso. Nodo lógico del grafo (agregadores, mediators).
    Virtual,
    /// Wrapper de daemon legacy. `fakes` activa shims D-Bus / sd_notify.
    Legacy {
        exec: String,
        argv: Vec<String>,
        fakes: BTreeSet<LegacyFacade>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LegacyFacade {
    SystemdLogind,
    SystemdHostnamed,
    SystemdNotify,
}

// =====================================================================
// Supervisión (heredada de arje)
// =====================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Supervision {
    Restart {
        #[serde(with = "duration_millis")]
        initial: Duration,
        #[serde(with = "duration_millis")]
        max: Duration,
    },
    OneShot,
    Delegate,
}

mod duration_millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(d)?;
        Ok(Duration::from_millis(ms))
    }
}

// =====================================================================
// Lifecycle / Priority (del modelo brahman)
// =====================================================================

/// Modelo de ejecución (rol). Ortogonal a `Supervision` (política de restart).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lifecycle {
    /// Servicio de larga duración.
    #[default]
    Daemon,
    /// Una sola ejecución; sale al terminar su tarea.
    Oneshot,
    /// Componente UI gestionado por el motor de widgets.
    Widget,
}

/// Tipo de relación entre dos Cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelationshipKind {
    /// Esta Card administra/posee al target (Ente sobre Mónada).
    Owns,
    /// Esta Card es administrada/poseída por el target (Mónada bajo Ente).
    OwnedBy,
    /// Esta Card procesa al target (Ente que consume Mónada).
    Processes,
    /// Esta Card es procesada por el target (Mónada siendo consumida).
    ProcessedBy,
    /// Relación lateral genérica.
    Sibling,
}

/// Referencia desde una Card a otra. Forma el grafo de relaciones del
/// fractal: "el Engine X posee la Mónada Y", "el Worker A procesa la
/// Tarea B", etc.
///
/// Es responsabilidad del que declara mantener `target_id` apuntando a
/// una Card que existe (o existió) en el ecosistema. El `target_label`
/// es redundante con el lookup en runtime, pero se incluye para que la
/// UI pueda renderear sin resolver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardReference {
    pub kind: RelationshipKind,
    pub target_id: Ulid,
    /// Label humano del target en el momento de declararse la
    /// referencia (cache; el target real puede haber cambiado de label).
    #[serde(default)]
    pub target_label: String,
}

/// Naturaleza de la entidad detrás de la Card.
///
/// La función de presentarse es la misma para todos: tener identidad,
/// resumen, capacidades, y poder ser encontrada por otros. Pero NO todas
/// las entidades son procesos — algunas son agrupaciones de datos
/// (Mónadas de Nouser, índices, streams).
///
/// El kind permite a consumidores (UI, broker, observadores) discriminar
/// sólo cuando importa, pero todos hablan el mismo protocolo de Card.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CardKind {
    /// Entidad runtime con `payload`/`soma`/`supervision` activos
    /// (proceso, módulo, daemon).
    #[default]
    Ente,
    /// Agrupación de datos sin proceso detrás (Mónadas Nouser, índices,
    /// resultados cacheados). `payload` típicamente `Virtual`.
    Data,
}

/// Faceta de datos: campos relevantes cuando `Card.kind != Ente`.
///
/// Optimizada para el wire — incluye sólo metadatos de presentación, NO
/// listas pesadas (los miembros, embeddings completos, etc. se consultan
/// al daemon dueño bajo demanda). El "presentation_hint" es un string
/// libre que la UI mapea a su lente (p. ej. `"code"` → editor de código).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DataFacet {
    /// Resumen humano (1-2 oraciones). Generado por el daemon dueño.
    #[serde(default)]
    pub summary: String,
    /// Tokens dominantes / palabras clave (5-10 típicamente).
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Centroide vectorial. Vacío si no hay embeddings calculados.
    #[serde(default)]
    pub centroid: Vec<f32>,
    /// Cantidad de elementos contenidos (archivos, registros, ...).
    #[serde(default)]
    pub member_count: u32,
    /// Métrica de dispersión interna [0, 1] (típicamente entropía).
    #[serde(default)]
    pub dispersion: f32,
    /// Hint de presentación. Strings libres como `"code"`, `"gallery"`,
    /// `"markdown"`, `"database"`, `"grid"`, `"tree"`. La UI los mapea.
    #[serde(default)]
    pub presentation_hint: String,
}

/// Prioridad de scheduling. Orden: `Low < Normal < High < Critical` —
/// usable como tiebreaker en el broker (mayor priority gana).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
    Critical,
}

/// Override per-contexto sobre los matches del broker.
///
/// La Card declara biases bajo `priority_contexts.<nombre>` que se
/// activan cuando el broker corre bajo ese contexto. Aplicación según rol:
///
/// - **Como consumidor**: `pin_to` sobreescribe el `pin_to` estático del
///   `Flow.pin_to` durante la búsqueda de productores.
/// - **Como productor**: `priority_offset` se suma a la priority base
///   (saturando en `[Low, Critical]`) para el ranking de candidatos.
///
/// Casos de uso típicos: test↔prod (mock vs real), foreground↔background
/// (latencia vs costo), trust gates (sólo productores con cierto nivel).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBias {
    /// Override del `pin_to` estático cuando el broker está en este
    /// contexto y la Card actúa como consumidor.
    ///
    /// **No se usa `skip_serializing_if` aquí**: postcard requiere
    /// layout fijo. La verbosidad extra en JSON (campos null/cero
    /// emitidos) es el costo aceptado para compatibilidad de wire.
    #[serde(default)]
    pub pin_to: Option<String>,

    /// Modifica la priority efectiva del Card como productor.
    /// `+1` lo eleva, `-1` lo baja. El resultado se clampa al rango de
    /// `Priority` ([Low, Critical]).
    #[serde(default)]
    pub priority_offset: i8,
}

// =====================================================================
// Flujos tipados (del modelo brahman)
// =====================================================================

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Flows {
    #[serde(default)]
    pub input: Vec<Flow>,
    #[serde(default)]
    pub output: Vec<Flow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    /// Nombre único dentro de su dirección.
    pub name: String,
    /// Tipo de los datos que viajan por el flujo.
    #[serde(rename = "type")]
    pub ty: TypeRef,
    /// Sugerencia de productor/consumidor concreto. El broker la respeta
    /// como pista; cae en matching por tipo si no es resoluble.
    #[serde(default)]
    pub pin_to: Option<String>,
}

/// Referencia a un tipo, discriminada para distinguir primitivas de tipos WIT.
///
/// **Wire format (JSON / TOML / postcard):** externally-tagged. Ejemplo JSON:
/// ```json
/// { "primitive": { "name": "string" } }
/// { "wit": { "package": "brahman:dht", "name": "entity-result" } }
/// ```
/// Se eligió externally-tagged por compatibilidad con postcard, que no
/// soporta `#[serde(tag = "...")]` (internally-tagged) en formatos no
/// self-describing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TypeRef {
    /// Tipo primitivo del runtime.
    Primitive { name: String },
    /// Tipo declarado en un paquete WIT.
    Wit {
        package: String,
        #[serde(default)]
        interface: Option<String>,
        name: String,
    },
}

// =====================================================================
// API: parseo y validación
// =====================================================================

impl Card {
    /// Construye una Card "viva" lista para registrarse en el broker:
    /// `id = Ulid::new()` (único), `label` provisto, todo lo demás en
    /// los defaults seguros (Payload::Virtual, Supervision::OneShot,
    /// CardKind::Ente, etc.).
    ///
    /// Diseñada para usarse en struct-literals con override parcial,
    /// igual que `Default` pero sin la trap de `Ulid::nil()`:
    ///
    /// ```ignore
    /// let card = Card {
    ///     kind: CardKind::Data,
    ///     payload: Payload::Embedded(serde_json::json!({"foo": 1})),
    ///     ..Card::new("mi-modulo.algo")
    /// };
    /// ```
    ///
    /// Para Cards de búsqueda/sentinel donde `nil` es semánticamente
    /// significativo, usá `Card::default()` directamente.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            id: Ulid::new(),
            label: label.into(),
            ..Self::default()
        }
    }

    /// Deserializa una Card desde JSON y valida.
    pub fn from_json(src: &str) -> Result<Self, CardError> {
        let c: Self = serde_json::from_str(src)?;
        c.validate()?;
        Ok(c)
    }

    /// Deserializa una Card desde TOML y valida.
    pub fn from_toml(src: &str) -> Result<Self, CardError> {
        let c: Self = toml::from_str(src)?;
        c.validate()?;
        Ok(c)
    }

    /// Carga una Card desde disco. Auto-detecta formato por extensión
    /// (`.json` o `.toml`).
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, CardError> {
        let p = path.as_ref();
        let src = std::fs::read_to_string(p)?;
        match p.extension().and_then(|e| e.to_str()) {
            Some("json") => Self::from_json(&src),
            Some("toml") => Self::from_toml(&src),
            _ => Err(CardError::UnknownFormat),
        }
    }

    /// Re-serializa la Card a JSON con indentación.
    pub fn to_json_pretty(&self) -> Result<String, CardError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Validación semántica exhaustiva, recursiva sobre `genesis`.
    pub fn validate(&self) -> Result<(), CardError> {
        if self.schema_version != CARD_SCHEMA_VERSION {
            return Err(CardError::SchemaMismatch {
                got: self.schema_version,
                expected: CARD_SCHEMA_VERSION,
            });
        }
        if self.label.is_empty() {
            return Err(CardError::EmptyLabel);
        }
        if self.label.len() > 256 {
            return Err(CardError::LabelTooLong(self.label.len()));
        }
        for cap in &self.requires {
            if self.provides.contains(cap) {
                return Err(CardError::SelfDependency(cap.clone()));
            }
        }
        validate_payload(&self.payload)?;
        validate_rlimits(&self.soma.rlimits)?;
        validate_cgroup(&self.soma.cgroup)?;
        check_unique_flow_names(&self.flow.input, "flow.input")?;
        check_unique_flow_names(&self.flow.output, "flow.output")?;
        for child in &self.genesis {
            child.validate()?;
        }
        Ok(())
    }
}

fn validate_payload(p: &Payload) -> Result<(), CardError> {
    match p {
        Payload::Native { exec, .. } | Payload::Legacy { exec, .. } => {
            if exec.trim().is_empty() {
                return Err(CardError::EmptyExec);
            }
        }
        Payload::Wasm { module_sha256, .. } => {
            if module_sha256.iter().all(|&b| b == 0) {
                return Err(CardError::SentinelWasmHash);
            }
        }
        Payload::Virtual => {}
    }
    Ok(())
}

fn validate_rlimits(rl: &ResourceLimits) -> Result<(), CardError> {
    if let Some(m) = rl.mem_bytes {
        if m == 0 {
            return Err(CardError::InvalidRlimit("mem_bytes=0"));
        }
        if m > 1u64 << 40 {
            return Err(CardError::InvalidRlimit("mem_bytes>1TiB"));
        }
    }
    if let Some(n) = rl.nproc {
        if n == 0 || n > 65535 {
            return Err(CardError::InvalidRlimit("nproc fuera de [1,65535]"));
        }
    }
    if let Some(n) = rl.nofile {
        if n == 0 || n > 1_048_576 {
            return Err(CardError::InvalidRlimit("nofile fuera de [1,1M]"));
        }
    }
    Ok(())
}

fn validate_cgroup(cg: &CgroupSpec) -> Result<(), CardError> {
    if let Some(w) = cg.cpu_weight {
        if !(1..=10000).contains(&w) {
            return Err(CardError::InvalidCgroupWeight("cpu_weight"));
        }
    }
    if let Some(w) = cg.io_weight {
        if !(1..=10000).contains(&w) {
            return Err(CardError::InvalidCgroupWeight("io_weight"));
        }
    }
    Ok(())
}

fn check_unique_flow_names(flows: &[Flow], section: &'static str) -> Result<(), CardError> {
    let mut seen = HashSet::new();
    for f in flows {
        if !seen.insert(f.name.as_str()) {
            return Err(CardError::DuplicateFlowName {
                section,
                name: f.name.clone(),
            });
        }
    }
    Ok(())
}

// =====================================================================
// Trust derivado
// =====================================================================

/// Nivel de confianza derivado de los permisos. **No es un campo declarado** —
/// se calcula. Una sola fuente de verdad: lo que el Admin concede.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrustLevel {
    /// Sin permisos — sandbox total.
    Untrusted,
    /// Permisos menores (loopback, FS read-only, IPC).
    Sandboxed,
    /// Permisos amplios (red saliente o FS read-write).
    Privileged,
    /// Capacidad de spawnear procesos.
    System,
}

impl TrustLevel {
    /// Política de derivación:
    /// - `processes = true` ⇒ `System`.
    /// - FS `read-write` o networking `outbound`/`full` ⇒ `Privileged`.
    /// - FS `read-only`, networking `loopback`, o cualquier IPC ⇒ `Sandboxed`.
    /// - Sin permisos ⇒ `Untrusted`.
    pub fn derive(p: &Permissions) -> Self {
        if p.processes {
            return Self::System;
        }
        if matches!(p.filesystem, FsPolicy::ReadWrite)
            || matches!(
                p.networking,
                NetworkingPolicy::Outbound | NetworkingPolicy::Full
            )
        {
            return Self::Privileged;
        }
        if matches!(p.filesystem, FsPolicy::ReadOnly)
            || matches!(p.networking, NetworkingPolicy::Loopback)
            || !p.ipc.allow.is_empty()
        {
            return Self::Sandboxed;
        }
        Self::Untrusted
    }
}

// =====================================================================
// Identidad runtime (Card + WIT extraído + trust)
// =====================================================================

/// Resumen de la interfaz WIT extraída del componente WASM/WIT.
/// Vacío para módulos agnósticos (sin contrato WIT).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitInterface {
    pub package: String,
    pub world: String,
    pub exports: Vec<String>,
    pub imports: Vec<String>,
}

/// Card resuelta a runtime: schema + WIT opcional + trust derivado.
/// Es lo que el Admin indexa.
#[derive(Debug, Clone)]
pub struct ResolvedCard {
    pub card: Card,
    /// `Some` si el módulo es consciente (expone WIT), `None` si es agnóstico.
    pub wit: Option<WitInterface>,
    pub trust: TrustLevel,
}

impl ResolvedCard {
    /// Construye una Card resuelta sin información WIT.
    pub fn from_agnostic(card: Card) -> Self {
        let trust = TrustLevel::derive(&card.permissions);
        Self {
            card,
            wit: None,
            trust,
        }
    }

    /// Construye una Card resuelta con interfaz WIT extraída.
    pub fn from_conscious(card: Card, wit: WitInterface) -> Self {
        let trust = TrustLevel::derive(&card.permissions);
        Self {
            card,
            wit: Some(wit),
            trust,
        }
    }
}

// =====================================================================
// WireCard — proyección postcard-friendly de Card
// =====================================================================

/// Forma de wire de [`Card`]: idéntica al schema rico **sin** el campo
/// `extensions` (incompatible con postcard porque `serde_json::Value`
/// usa secuencias/maps de longitud dinámica).
///
/// Conversión:
/// - `WireCard::from(card)` descarta `extensions` y proyecta `genesis`
///   recursivamente.
/// - `Card::from(wire)` recupera todos los campos; `extensions` queda
///   vacío (la información de extensions no cruza el wire).
///
/// Esta separación implementa el contrato:
/// - **JSON/TOML**: `Card` directa, con extensiones preservadas.
/// - **Wire (postcard)**: `WireCard`, sin extensiones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireCard {
    pub schema_version: u16,
    pub id: Ulid,
    #[serde(default)]
    pub lineage: Option<Ulid>,
    pub label: String,
    #[serde(default)]
    pub provides: BTreeSet<Capability>,
    #[serde(default)]
    pub requires: BTreeSet<Capability>,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub soma: SomaSpec,
    pub payload: Payload,
    pub supervision: Supervision,
    #[serde(default)]
    pub lifecycle: Lifecycle,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub flow: Flows,
    #[serde(default)]
    pub genesis: Vec<WireCard>,
    #[serde(default)]
    pub service_socket: Option<PathBuf>,
    #[serde(default)]
    pub references: Vec<CardReference>,
    #[serde(default)]
    pub kind: CardKind,
    #[serde(default)]
    pub data: Option<DataFacet>,
    #[serde(default)]
    pub priority_contexts: BTreeMap<String, ContextBias>,
}

impl From<Card> for WireCard {
    fn from(c: Card) -> Self {
        Self {
            schema_version: c.schema_version,
            id: c.id,
            lineage: c.lineage,
            label: c.label,
            provides: c.provides,
            requires: c.requires,
            permissions: c.permissions,
            soma: c.soma,
            payload: c.payload,
            supervision: c.supervision,
            lifecycle: c.lifecycle,
            priority: c.priority,
            flow: c.flow,
            genesis: c.genesis.into_iter().map(WireCard::from).collect(),
            service_socket: c.service_socket,
            references: c.references,
            kind: c.kind,
            data: c.data,
            priority_contexts: c.priority_contexts,
        }
    }
}

impl From<WireCard> for Card {
    fn from(w: WireCard) -> Self {
        Self {
            schema_version: w.schema_version,
            id: w.id,
            lineage: w.lineage,
            label: w.label,
            provides: w.provides,
            requires: w.requires,
            permissions: w.permissions,
            soma: w.soma,
            payload: w.payload,
            supervision: w.supervision,
            lifecycle: w.lifecycle,
            priority: w.priority,
            flow: w.flow,
            genesis: w.genesis.into_iter().map(Card::from).collect(),
            service_socket: w.service_socket,
            references: w.references,
            kind: w.kind,
            data: w.data,
            priority_contexts: w.priority_contexts,
            extensions: BTreeMap::new(),
        }
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_card_json() -> &'static str {
        r#"{
            "schema_version": 1,
            "id": "01HQAR53D4M2NBV8KZTYXFGS01",
            "lineage": null,
            "label": "brahman.semantic_dht",
            "provides": ["Spawn", "Journal"],
            "requires": [],
            "permissions": {
                "networking": "loopback",
                "filesystem": "read-only",
                "ipc": { "allow": ["wit-v1"] },
                "processes": false
            },
            "soma": {
                "namespaces": {
                    "mount": false, "pid": false, "net": false,
                    "uts": false, "ipc": false, "user": false, "cgroup": false
                },
                "rlimits": { "mem_bytes": null, "nproc": null, "nofile": null },
                "cgroup": { "path": "ente.slice/dht", "cpu_weight": null, "io_weight": null },
                "cpu_affinity": null
            },
            "payload": "Virtual",
            "supervision": { "Restart": { "initial": 100, "max": 30000 } },
            "lifecycle": "daemon",
            "priority": "high",
            "flow": {
                "input": [
                    { "name": "search-query", "type": { "primitive": { "name": "string" } } }
                ],
                "output": [
                    { "name": "dht-results",
                      "type": { "wit": { "package": "brahman:dht", "name": "entity-result" } } }
                ]
            },
            "genesis": []
        }"#
    }

    #[test]
    fn parses_full_json() {
        let c = Card::from_json(sample_card_json()).unwrap();
        assert_eq!(c.label, "brahman.semantic_dht");
        assert_eq!(c.lifecycle, Lifecycle::Daemon);
        assert_eq!(c.priority, Priority::High);
        assert_eq!(c.permissions.filesystem, FsPolicy::ReadOnly);
        assert_eq!(c.permissions.networking, NetworkingPolicy::Loopback);
        assert_eq!(c.permissions.ipc.allow, vec!["wit-v1".to_string()]);
        assert_eq!(c.flow.input.len(), 1);
        assert_eq!(c.flow.output.len(), 1);
        match &c.flow.output[0].ty {
            TypeRef::Wit { package, name, .. } => {
                assert_eq!(package, "brahman:dht");
                assert_eq!(name, "entity-result");
            }
            _ => panic!("expected Wit"),
        }
    }

    #[test]
    fn json_roundtrip_preserves_shape() {
        let c1 = Card::from_json(sample_card_json()).unwrap();
        let s = c1.to_json_pretty().unwrap();
        let c2 = Card::from_json(&s).unwrap();
        assert_eq!(c1.label, c2.label);
        assert_eq!(c1.flow.input.len(), c2.flow.input.len());
    }

    #[test]
    fn trust_derivation() {
        let mut p = Permissions::default();
        assert_eq!(TrustLevel::derive(&p), TrustLevel::Untrusted);
        p.filesystem = FsPolicy::ReadOnly;
        assert_eq!(TrustLevel::derive(&p), TrustLevel::Sandboxed);
        p.networking = NetworkingPolicy::Outbound;
        assert_eq!(TrustLevel::derive(&p), TrustLevel::Privileged);
        p.processes = true;
        assert_eq!(TrustLevel::derive(&p), TrustLevel::System);
    }

    #[test]
    fn duplicate_flow_names_rejected() {
        let mut c: Card = serde_json::from_str(sample_card_json()).unwrap();
        c.flow.input.push(c.flow.input[0].clone());
        assert!(matches!(
            c.validate(),
            Err(CardError::DuplicateFlowName { .. })
        ));
    }

    #[test]
    fn self_dependency_rejected() {
        let mut c: Card = serde_json::from_str(sample_card_json()).unwrap();
        c.requires.insert(Capability::Spawn);
        assert!(matches!(c.validate(), Err(CardError::SelfDependency(_))));
    }

    #[test]
    fn invalid_genesis_propagates() {
        let parent_src = r#"{
            "schema_version": 1,
            "id": "01HQAR53D4M2NBV8KZTYXFGS01",
            "label": "parent",
            "provides": [], "requires": [],
            "soma": {
                "namespaces": {"mount":false,"pid":false,"net":false,"uts":false,"ipc":false,"user":false,"cgroup":false},
                "rlimits": {"mem_bytes":null,"nproc":null,"nofile":null},
                "cgroup": {"path":"x","cpu_weight":null,"io_weight":null},
                "cpu_affinity": null
            },
            "payload": "Virtual",
            "supervision": "OneShot",
            "genesis": [{
                "schema_version": 1,
                "id": "01HQAR53D4M2NBV8KZTYXFGS02",
                "label": "",
                "provides": [], "requires": [],
                "soma": {
                    "namespaces": {"mount":false,"pid":false,"net":false,"uts":false,"ipc":false,"user":false,"cgroup":false},
                    "rlimits": {"mem_bytes":null,"nproc":null,"nofile":null},
                    "cgroup": {"path":"x","cpu_weight":null,"io_weight":null},
                    "cpu_affinity": null
                },
                "payload": "Virtual",
                "supervision": "OneShot",
                "genesis": []
            }]
        }"#;
        assert!(matches!(
            Card::from_json(parent_src),
            Err(CardError::EmptyLabel)
        ));
    }

    #[test]
    fn presentation_card_carries_derived_trust() {
        let c = Card::from_json(sample_card_json()).unwrap();
        let resolved = ResolvedCard::from_agnostic(c);
        assert_eq!(resolved.trust, TrustLevel::Sandboxed);
        assert!(resolved.wit.is_none());
    }

    #[test]
    fn arje_seed_format_compatible() {
        // Reproduce el formato canónico de arje (sin lifecycle/priority/flow,
        // que son aditivos brahman). Debe parsear con defaults.
        let src = r#"{
            "schema_version": 1,
            "id": "01HQAR53D4M2NBV8KZTYXFGS01",
            "lineage": null,
            "label": "vps-min",
            "provides": ["Spawn", "Journal"],
            "requires": [],
            "soma": {
                "namespaces": {"mount":false,"pid":false,"net":false,"uts":false,"ipc":false,"user":false,"cgroup":false},
                "rlimits": {"mem_bytes":null,"nproc":null,"nofile":null},
                "cgroup": {"path":"ente.slice/zero","cpu_weight":null,"io_weight":null},
                "cpu_affinity": null
            },
            "payload": "Virtual",
            "supervision": "OneShot",
            "genesis": []
        }"#;
        let c = Card::from_json(src).unwrap();
        assert_eq!(c.lifecycle, Lifecycle::Daemon); // default
        assert_eq!(c.priority, Priority::Normal); // default
        assert_eq!(c.flow.input.len(), 0);
    }

    #[test]
    fn extensions_preserved_in_json_roundtrip() {
        let src = r#"{
            "schema_version": 1,
            "id": "01HQAR53D4M2NBV8KZTYXFGS01",
            "label": "x",
            "soma": {
                "namespaces": {"mount":false,"pid":false,"net":false,"uts":false,"ipc":false,"user":false,"cgroup":false},
                "rlimits": {"mem_bytes":null,"nproc":null,"nofile":null},
                "cgroup": {"path":"x","cpu_weight":null,"io_weight":null},
                "cpu_affinity": null
            },
            "payload": "Virtual",
            "supervision": "OneShot",
            "author": "sergio",
            "tags": ["draft", "experimental"]
        }"#;
        let c = Card::from_json(src).unwrap();
        assert_eq!(c.extensions.get("author").and_then(|v| v.as_str()), Some("sergio"));
        assert!(c.extensions.contains_key("tags"));

        // Roundtrip JSON: extensions deben re-emitirse.
        let s = c.to_json_pretty().unwrap();
        let c2 = Card::from_json(&s).unwrap();
        assert_eq!(c2.extensions.get("author"), c.extensions.get("author"));
    }

    #[test]
    fn wire_card_roundtrip_strips_extensions() {
        let src = r#"{
            "schema_version": 1,
            "id": "01HQAR53D4M2NBV8KZTYXFGS01",
            "label": "x",
            "soma": {
                "namespaces": {"mount":false,"pid":false,"net":false,"uts":false,"ipc":false,"user":false,"cgroup":false},
                "rlimits": {"mem_bytes":null,"nproc":null,"nofile":null},
                "cgroup": {"path":"x","cpu_weight":null,"io_weight":null},
                "cpu_affinity": null
            },
            "payload": "Virtual",
            "supervision": "OneShot",
            "author": "sergio"
        }"#;
        let c = Card::from_json(src).unwrap();
        assert!(c.extensions.contains_key("author"));

        // Card → WireCard descarta extensions.
        let wire: WireCard = c.into();
        assert_eq!(wire.label, "x");

        // WireCard → Card → extensiones quedan vacías (se perdieron).
        let c_back: Card = wire.into();
        assert_eq!(c_back.label, "x");
        assert!(c_back.extensions.is_empty(), "extensions sobreviven al wire");
    }

    #[test]
    fn wirecard_postcard_with_priority_contexts() {
        // Repro del bug que rompía chasqui-nous-mock: ContextBias con
        // skip_serializing_if causaba que postcard leyera bytes
        // equivocados. Sin esos atributos, el roundtrip es estable.
        let src = r#"{
            "schema_version": 1,
            "id": "01HQAR53D4M2NBV8KZTYXFGS01",
            "label": "x",
            "soma": {
                "namespaces": {"mount":false,"pid":false,"net":false,"uts":false,"ipc":false,"user":false,"cgroup":false},
                "rlimits": {"mem_bytes":null,"nproc":null,"nofile":null},
                "cgroup": {"path":"x","cpu_weight":null,"io_weight":null},
                "cpu_affinity": null
            },
            "payload": "Virtual",
            "supervision": "OneShot"
        }"#;
        let mut c = Card::from_json(src).unwrap();
        c.priority_contexts.insert(
            "test".into(),
            ContextBias {
                pin_to: None,
                priority_offset: 1,
            },
        );
        c.priority_contexts.insert(
            "prod".into(),
            ContextBias {
                pin_to: Some("real-nous".into()),
                priority_offset: 2,
            },
        );

        let wire: WireCard = c.into();
        let bytes = postcard::to_allocvec(&wire).expect("postcard encode");
        let decoded: WireCard = postcard::from_bytes(&bytes).expect("postcard decode");

        assert_eq!(decoded.priority_contexts.len(), 2);
        let test_bias = decoded
            .priority_contexts
            .get("test")
            .expect("test context");
        assert_eq!(test_bias.priority_offset, 1);
        assert!(test_bias.pin_to.is_none());
        let prod_bias = decoded
            .priority_contexts
            .get("prod")
            .expect("prod context");
        assert_eq!(prod_bias.pin_to.as_deref(), Some("real-nous"));
        assert_eq!(prod_bias.priority_offset, 2);
    }

    #[test]
    fn wire_card_postcard_friendly() {
        // Validación: WireCard puede ser postcard-encoded sin error.
        // Si Card tuviera extensions populadas, el encode rompería con
        // "length of a sequence must be known". WireCard las descarta.
        let src = r#"{
            "schema_version": 1,
            "id": "01HQAR53D4M2NBV8KZTYXFGS01",
            "label": "x",
            "soma": {
                "namespaces": {"mount":false,"pid":false,"net":false,"uts":false,"ipc":false,"user":false,"cgroup":false},
                "rlimits": {"mem_bytes":null,"nproc":null,"nofile":null},
                "cgroup": {"path":"x","cpu_weight":null,"io_weight":null},
                "cpu_affinity": null
            },
            "payload": "Virtual",
            "supervision": "OneShot",
            "author": "sergio"
        }"#;
        let c = Card::from_json(src).unwrap();
        let wire: WireCard = c.into();
        let bytes = postcard::to_allocvec(&wire).expect("WireCard debe encodear");
        let decoded: WireCard = postcard::from_bytes(&bytes).expect("WireCard debe decodear");
        assert_eq!(decoded.label, "x");
    }

    #[test]
    fn new_assigns_real_ulid_and_label() {
        let c = Card::new("chasqui.engine");
        assert_eq!(c.label, "chasqui.engine");
        assert_ne!(c.id, Ulid::nil(), "Card::new no debe dejar id en nil");
    }

    #[test]
    fn new_yields_distinct_ids_per_call() {
        let a = Card::new("x");
        let b = Card::new("x");
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn default_keeps_nil_id_for_struct_update_pattern() {
        // Mantener este invariante explícito: Default::default() es
        // determinista y devuelve nil. Cualquier cambio aquí rompería
        // el patrón `..Default::default()` en patterns de búsqueda.
        let d = Card::default();
        assert_eq!(d.id, Ulid::nil());
        assert!(d.label.is_empty());
    }
}
