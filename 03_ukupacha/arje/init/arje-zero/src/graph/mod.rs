//! `EnteGraph`: estado del fractal vivo en PID 1.
//!
//! Diseño:
//!   - Submódulos por concern: lifecycle, topology, shutdown, bus_mediator,
//!     devices, capabilities. Cada uno extiende `impl EnteGraph` con métodos
//!     relacionados.
//!   - Estado plano (no substructs todavía) — la separación es por
//!     comportamiento, no por compartimentación de datos.
//!   - Toda mutación pasa por el bucle primordial vía `GraphEvent`. Los
//!     submódulos se llaman desde `main.rs::primordial_loop`.

mod bus_mediator;
mod capabilities;
mod devices;
mod lifecycle;
mod shutdown;
mod topology;

use arje_bus::{BusMessage, BusResponse};
use arje_card::{Capability, EntityCard};
use nix::unistd::Pid;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};
use ulid::Ulid;

// `SHUTDOWN_GRACE` está re-exportado bajo `crate::graph::shutdown::SHUTDOWN_GRACE`
// directo; la re-export adicional aquí no se usa todavía.

/// Bit alto encendido en `seq` para invokes server-iniciados — evita choque
/// con secuencias allocadas por clientes.
pub(in crate::graph) const SERVER_SEQ_FLAG: u64 = 1u64 << 63;

pub struct EnteGraph {
    pub(in crate::graph) seed: EntityCard,
    /// Entes encarnados como proceso o nodo virtual. id↔pid bidireccional.
    pub(in crate::graph) incarnated: HashMap<Ulid, Incarnated>,
    pub(in crate::graph) by_pid: HashMap<i32, Ulid>,
    /// Quién provee qué capacidad. Resuelve `requires` y `pick_invokable`.
    pub(in crate::graph) providers: BTreeMap<Capability, BTreeSet<Ulid>>,
    /// Tokens de capability emitidos. Revocables al morir el proveedor.
    pub(in crate::graph) next_token: u64,
    pub(in crate::graph) grants: HashMap<u64, GrantedCapability>,
    /// Dispositivos del kernel presentes (devpath → última UEvent).
    pub(in crate::graph) devices: HashMap<String, arje_kernel::UEvent>,
    /// Cards genesis pendientes de instanciar (extraídas de la Semilla).
    pub(in crate::graph) pending_genesis: Vec<EntityCard>,
    /// Hijos directos por lineage. parent → [child, ...].
    pub(in crate::graph) children: HashMap<Ulid, Vec<Ulid>>,
    /// Conexiones del bus indexadas por la identidad anunciada y verificada
    /// con SO_PEERCRED. El value es el extremo de escritura del writer task.
    pub(in crate::graph) bus_connections: HashMap<Ulid, mpsc::Sender<BusMessage>>,
    /// Conexiones suscritas al stream de eventos de ciclo de vida
    /// (`BusRequest::Subscribe`). A diferencia de `bus_connections`, no se
    /// indexan por identidad —un suscriptor puede ser anónimo, como un
    /// supervisor externo (la capa de IA de hammer). Se purgan al difundir
    /// cuando el receptor cerró su extremo. Ver `on_death` → `broadcast_lifecycle`.
    pub(in crate::graph) lifecycle_subscribers: Vec<mpsc::Sender<BusMessage>>,
    /// Invokes forwardeados pendientes de respuesta del proveedor.
    pub(in crate::graph) pending_invokes: HashMap<u64, oneshot::Sender<BusResponse>>,
    pub(in crate::graph) next_invoke_seq: u64,
    /// Estado de supervisión por `card.label`. Sobrevive a la rotación de
    /// Ulid del Restart — el "thread" de identidad es el label, no el id.
    pub(in crate::graph) restart_state: HashMap<String, RestartState>,
    /// Inhibiciones declaradas por el cerebro. Key: reason. Value: instante
    /// de expiración. Cualquier acción escalatoria (power-mgmt, BrainInvoke,
    /// BrainNotify, BrainSpawn) se descarta mientras el set no esté vacío.
    pub(in crate::graph) inhibits: BTreeMap<String, Instant>,
}

/// TTL fijo para inhibiciones del cerebro. Suficiente largo para cubrir un
/// período de turbulencia (30s) sin perpetuar el estado si el cerebro deja
/// de re-afirmar la regla.
pub(in crate::graph) const INHIBIT_TTL: std::time::Duration =
    std::time::Duration::from_secs(30);

#[derive(Default, Debug)]
pub(in crate::graph) struct RestartState {
    /// Backoff exponencial canónico (`sandokan-lifecycle`). `None` hasta la
    /// primera muerte: se construye con `(initial, max)` de la Supervision de
    /// la Card. Se resetea cuando un Ente vive lo suficiente para considerarse
    /// estable (≥ `max`). Antes esto era un `attempts: u32` + un `backoff_delay`
    /// propio — duplicaba la política; ver `shared/sandokan/SDD.md` §5 Fase 1.
    pub backoff: Option<sandokan_lifecycle::Backoff>,
    /// Instante en que el último spawn arrancó. None = nunca encarnado.
    pub last_started_at: Option<Instant>,
    /// Restarts acumulados desde el último período estable. Se expone como
    /// telemetría de ciclo de vida (EnteTelemetry → SDD §6 Fase 2).
    pub restarts: u32,
}

pub(in crate::graph) struct Incarnated {
    pub card: EntityCard,
    pub pid: Option<Pid>,
    /// Capacidades añadidas en runtime vía BusRequest::UpdateCapabilities.
    /// La Card original es immutable; la "vista efectiva" del Ente es
    /// `card.provides ∪ dynamic_provides`.
    pub dynamic_provides: BTreeSet<Capability>,
}

pub(in crate::graph) struct GrantedCapability {
    pub cap: Capability,
    pub provider: Ulid,
    pub holder: Ulid,
    /// Instante en el que el grant deja de ser válido. El garbage collector
    /// del cerebro purga grants con `Instant::now() > expires_at`.
    pub expires_at: std::time::Instant,
}

/// TTL default para grants cuando la cap no tiene override. 60s es un
/// compromiso: largo enough para evitar churn en patrones interactivos,
/// corto enough para que credenciales filtradas expiren rápidamente.
///
/// Reservado para el flujo de capability granting (no cableado todavía).
#[allow(dead_code)]
pub const DEFAULT_GRANT_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Quota máxima de tokens activos por (holder, cap). Caps escaladas tienen
/// quota baja para limitar fugas de credenciales; caps de uso frecuente
/// (Endpoint, Journal) son más laxas.
pub fn quota_for_capability(cap: &Capability) -> u32 {
    match cap {
        // Caps escaladas: pocos tokens, fuerza patrón request-act-release.
        Capability::Spawn => 2,
        Capability::FilesystemRoot => 2,
        Capability::Device { .. } => 4,
        // Caps de propósito general.
        Capability::Endpoint { .. } => 16,
        Capability::KernelNetlink(_) => 4,
        Capability::LegacyLogind => 8,
        // Logging: hasta 32 streams.
        Capability::Journal => 32,
    }
}

/// TTL específico por variante de Capability. Caps de mayor riesgo / costo
/// (Spawn, FilesystemRoot) tienen TTL más corto; caps "logging" como
/// Journal pueden vivir más.
///
/// Cualquier cap no listada cae al `DEFAULT_GRANT_TTL`.
pub fn ttl_for_capability(cap: &Capability) -> std::time::Duration {
    use std::time::Duration;
    match cap {
        // Caps escaladas: TTL corto para forzar renovación frecuente.
        Capability::Spawn => Duration::from_secs(30),
        Capability::FilesystemRoot => Duration::from_secs(30),
        Capability::Device { .. } => Duration::from_secs(60),
        // Caps de propósito general.
        Capability::Endpoint { .. } => Duration::from_secs(300),  // 5 min
        Capability::KernelNetlink(_) => Duration::from_secs(300),
        Capability::LegacyLogind => Duration::from_secs(300),
        // Logging puede vivir mucho.
        Capability::Journal => Duration::from_secs(3600),  // 1h
    }
}

impl EnteGraph {
    pub fn new(mut seed: EntityCard) -> Self {
        // Extraemos genesis antes de almacenar la Semilla — evita duplicación
        // en `incarnated[seed.id]`.
        let pending_genesis = std::mem::take(&mut seed.genesis);
        let mut g = Self {
            seed: seed.clone(),
            incarnated: HashMap::new(),
            by_pid: HashMap::new(),
            providers: BTreeMap::new(),
            next_token: 1,
            grants: HashMap::new(),
            devices: HashMap::new(),
            pending_genesis,
            children: HashMap::new(),
            bus_connections: HashMap::new(),
            lifecycle_subscribers: Vec::new(),
            pending_invokes: HashMap::new(),
            next_invoke_seq: 0,
            restart_state: HashMap::new(),
            inhibits: BTreeMap::new(),
        };
        // El Ente #0 se inscribe a sí mismo como proveedor de las capacidades
        // que su Card declara — sólo así los hijos pueden requerirlas.
        g.register_provider(&seed);
        g.incarnated.insert(seed.id, Incarnated {
            card: seed, pid: None,
            dynamic_provides: BTreeSet::new(),
        });
        g
    }

    pub fn lookup_pid(&self, pid: Pid) -> Option<Ulid> {
        self.by_pid.get(&pid.as_raw()).copied()
    }

    /// Acceso read-only a la Card de un Ente vivo. Usado por el cerebro
    /// para hidratar `SubjectInfo` sin clonar todo el mapa.
    pub fn peek_card(&self, id: &Ulid) -> Option<&EntityCard> {
        self.incarnated.get(id).map(|i| &i.card)
    }

    /// Captura el estado live como snapshot serializable. Excluye la Semilla
    /// (será re-sintetizada al restore con su seed_id preservado).
    pub fn snapshot(&self) -> arje_snapshot::FractalSnapshot {
        let entes: Vec<EntityCard> = self.incarnated.iter()
            .filter(|(id, _)| **id != self.seed.id)
            .map(|(_, inc)| inc.card.clone())
            .collect();
        arje_snapshot::FractalSnapshot {
            version: arje_snapshot::SNAPSHOT_VERSION,
            timestamp_ms: arje_snapshot::now_ms(),
            seed_id: self.seed.id,
            seed_label: self.seed.label.clone(),
            entes,
        }
    }

    pub(in crate::graph) fn register_provider(&mut self, card: &EntityCard) {
        for cap in &card.provides {
            self.providers.entry(cap.clone()).or_default().insert(card.id);
        }
    }

    pub(in crate::graph) fn unregister_provider(&mut self, card: &EntityCard) {
        for cap in &card.provides {
            if let Some(set) = self.providers.get_mut(cap) {
                set.remove(&card.id);
            }
        }
        // Revocar grants emitidos por el Ente fallecido.
        let revoked: Vec<u64> = self.grants.iter()
            .filter(|(_, g)| g.provider == card.id)
            .map(|(t, _)| *t)
            .collect();
        for t in revoked {
            self.grants.remove(&t);
        }
    }

    /// Quita una capacidad dinámica del índice de providers para un Ente
    /// específico. Usado al recibir UpdateCapabilities con `removes`.
    pub(in crate::graph) fn unregister_dynamic_cap(&mut self, ente_id: Ulid, cap: &Capability) {
        if let Some(set) = self.providers.get_mut(cap) {
            set.remove(&ente_id);
        }
    }

    /// Inserta una capacidad dinámica al índice de providers para un Ente.
    pub(in crate::graph) fn register_dynamic_cap(&mut self, ente_id: Ulid, cap: Capability) {
        self.providers.entry(cap).or_default().insert(ente_id);
    }

    /// El Ente #0 (semilla) tiene todas sus capacidades declaradas. Otros
    /// las tienen si su Card las declara o si poseen un grant vivo.
    pub(in crate::graph) fn holder_has(&self, holder: Ulid, cap: &Capability) -> bool {
        if holder == self.seed.id {
            return self.seed.provides.contains(cap);
        }
        if let Some(inc) = self.incarnated.get(&holder) {
            if inc.card.provides.contains(cap) { return true; }
        }
        self.grants.values().any(|g| g.holder == holder && &g.cap == cap)
    }
}
