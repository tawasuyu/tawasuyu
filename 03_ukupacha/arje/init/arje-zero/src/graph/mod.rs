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
pub(crate) mod resolve;
mod shutdown;
mod topology;

use arje_bus::{BusMessage, BusResponse};
use arje_card::{Capability, EntityCard, Payload};
use nix::unistd::Pid;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    /// Entes marcados para detención a pedido (teardown de bundle): cuando su
    /// muerte llegue por SIGCHLD, `on_death` salta el restart y los baja de
    /// verdad en vez de revivirlos. Simétrico a `SpawnCardFromDisk`.
    pub(in crate::graph) stopping: HashSet<Ulid>,
    /// Entes `Restart` que cayeron porque su "piso" desapareció (una capability
    /// de la que dependen dejó de tener proveedor — p. ej. el compositor murió y
    /// se llevó a sus clientes). En vez de descartarlos, quedan acá ESPERANDO; en
    /// cuanto el proveedor reaparece (el piso vuelve), `drain_refloorable` los
    /// re-spawnea en orden topológico. Uno por `label` (la última encarnación
    /// gana). Ver SDD §re-floor.
    pub(in crate::graph) parked: Vec<EntityCard>,
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
            stopping: HashSet::new(),
            parked: Vec::new(),
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

    /// Raíces REALES del CAS para el GC: todo objeto del CAS al que el fractal
    /// vivo todavía apunta. Camina la Semilla (con su árbol genesis) y cada Card
    /// encarnada, recolectando:
    ///   - **Wasm**: `module_sha256` de cada `Payload::Wasm` (el bytecode que el
    ///     Ente ejecuta — borrarlo rompería un respawn).
    ///   - **Atestación**: cada `attest[].bytecode` (los binarios cosechados al
    ///     CAS por `--harvest-cas`; borrarlos rompería la distribución por AoE y
    ///     la reproducción remota del sistema).
    /// El GC une esto con la cadena de audit (desde el head) — ver
    /// `arje_brain::IntrospectRequest::GcCas`. Sin esto, un `gc-cas` con
    /// `extra_roots` vacío barrería Wasm y binarios todavía en uso.
    pub fn cas_roots(&self) -> HashSet<[u8; 32]> {
        fn walk(card: &EntityCard, roots: &mut HashSet<[u8; 32]>) {
            if let Payload::Wasm { module_sha256, .. } = &card.payload {
                roots.insert(*module_sha256);
            }
            for c in &card.attest {
                roots.insert(c.bytecode);
            }
            for hija in &card.genesis {
                walk(hija, roots);
            }
        }
        let mut roots = HashSet::new();
        walk(&self.seed, &mut roots);
        // `EnteGraph::new` mueve el genesis de la Semilla a `pending_genesis`
        // (queda fuera de `seed.genesis`), así que hay que caminarlo aparte: sus
        // Wasm/bytecodes son raíces aunque el Ente no haya encarnado todavía.
        for card in &self.pending_genesis {
            walk(card, &mut roots);
        }
        for inc in self.incarnated.values() {
            walk(&inc.card, &mut roots);
        }
        roots
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
            // Preservar la atestación de la Semilla: sin esto el restore
            // dejaría el seed sin gate (ver doctrina en arje-snapshot).
            attest: self.seed.attest.clone(),
            attest_rootkey: self.seed.attest_rootkey,
            attest_policy: self.seed.attest_policy,
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

    /// Id de la Semilla (Ente #0). Autorizada para `Capability::Spawn`, es el
    /// requester de los re-spawns internos (restart, re-floor).
    pub(crate) fn seed_id(&self) -> Ulid {
        self.seed.id
    }

    /// Aparca un Ente que no pudo arrancar porque su piso (una capability de la
    /// que depende) no está disponible. Dedup por `label`: la última encarnación
    /// reemplaza a la previa aparcada (el "hilo" de identidad es el label, igual
    /// que `restart_state`).
    pub(in crate::graph) fn park_ente(&mut self, card: EntityCard) {
        self.parked.retain(|c| c.label != card.label);
        self.parked.push(card);
    }

    /// Saca de `parked` las Cards cuyos contratos YA se satisfacen con las
    /// capacidades disponibles, ordenadas topológicamente (proveedor antes que
    /// consumidor) por `resolve::plan_spawn`. Las que siguen sin piso quedan
    /// aparcadas. El caller las re-encola como `SpawnRequest` por el canal (no
    /// reentrante). Esto es "volver a poner el piso": cuando el compositor
    /// reaparece, sus clientes caídos vuelven solos y en orden.
    pub(crate) fn drain_refloorable(&mut self) -> Vec<EntityCard> {
        if self.parked.is_empty() {
            return Vec::new();
        }
        let available = self.available_caps();
        let mut ready = Vec::new();
        let mut still = Vec::new();
        for card in std::mem::take(&mut self.parked) {
            if card.deps_satisfied(&available).is_ok() {
                ready.push(card);
            } else {
                still.push(card);
            }
        }
        self.parked = still;
        // Orden topológico entre los que vuelven (uno puede proveer el piso de
        // otro). `external` = lo ya disponible.
        let plan = resolve::plan_spawn(&ready, &available);
        let ordered: Vec<EntityCard> = plan.order.iter().map(|&i| ready[i].clone()).collect();
        // Cualquiera que el plan rechace (ciclo entre los que vuelven) se
        // re-aparca para reintentar cuando cambie el piso.
        for (i, _) in &plan.rejected {
            self.parked.push(ready[*i].clone());
        }
        ordered
    }

    /// Conjunto de capacidades actualmente DISPONIBLES en el fractal: las que
    /// tiene al menos un proveedor vivo. Filtra entradas con set vacío (un
    /// proveedor que murió deja la key con set vacío hasta el próximo barrido).
    /// Es lo que se evalúa contra los contratos de una Card al spawnear.
    pub(in crate::graph) fn available_caps(&self) -> BTreeSet<Capability> {
        self.providers
            .iter()
            .filter(|(_, holders)| !holders.is_empty())
            .map(|(cap, _)| cap.clone())
            .collect()
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

#[cfg(test)]
mod cas_roots_tests {
    use super::*;

    fn concesion(bytecode: [u8; 32]) -> arje_attest::ConcesionCapacidad {
        arje_attest::ConcesionCapacidad {
            bytecode,
            permisos: 0,
            autor: [0u8; 32],
            firma: [0u8; 64],
        }
    }

    #[test]
    fn cas_roots_recoge_wasm_y_bytecodes_del_attest() {
        let mut seed = EntityCard::new("seed");
        // Binarios cosechados → bytecodes en el attest del seed.
        seed.attest = vec![concesion([1u8; 32]), concesion([2u8; 32])];
        // Un Ente Wasm en el genesis → su module_sha256 es raíz.
        let mut wasm_ente = EntityCard::new("wasm-ente");
        wasm_ente.payload = Payload::Wasm { module_sha256: [3u8; 32], entry: "_start".into() };
        seed.genesis.push(wasm_ente);

        let mut graph = EnteGraph::new(seed);
        // Un Ente Wasm encarrnado dinámicamente (p. ej. RunCard) también cuenta.
        let mut dyn_card = EntityCard::new("dyn-wasm");
        dyn_card.payload = Payload::Wasm { module_sha256: [4u8; 32], entry: "_start".into() };
        let id = dyn_card.id;
        graph.incarnated.insert(id, Incarnated {
            card: dyn_card,
            pid: None,
            dynamic_provides: Default::default(),
        });

        let roots = graph.cas_roots();
        for r in [[1u8; 32], [2u8; 32], [3u8; 32], [4u8; 32]] {
            assert!(roots.contains(&r), "falta la raíz {r:?}");
        }
        assert_eq!(roots.len(), 4, "exactamente las 4 raíces vivas, sin basura");
    }
}

#[cfg(test)]
mod refloor_tests {
    //! "Volver a poner el piso": un cliente `Restart` que depende de una
    //! capability-piso (la del compositor) se APARCA si el piso no está, y
    //! revive en cuanto el proveedor reaparece.
    use super::*;
    use arje_card::{Capability, EntityCard, InterfaceId, Supervision};
    use std::time::Duration;

    fn seed_con_spawn() -> EntityCard {
        let mut seed = EntityCard::new("seed");
        seed.provides = [Capability::Spawn].into_iter().collect();
        seed
    }

    fn piso() -> Capability {
        Capability::Endpoint {
            interface: InterfaceId([7u8; 16]),
            version: 1,
        }
    }

    #[tokio::test]
    async fn cliente_sin_piso_se_aparca_y_revive_cuando_vuelve() {
        let mut g = EnteGraph::new(seed_con_spawn());
        let seed_id = g.seed_id();

        // Cliente Restart que REQUIERE el piso (el endpoint del compositor).
        let mut client = EntityCard::new("cliente-gui");
        client.requires = [piso()].into_iter().collect();
        client.supervision = Supervision::Restart {
            initial: Duration::from_millis(10),
            max: Duration::from_secs(1),
        };
        g.authorize_and_spawn(client, seed_id).await.unwrap();

        // Sin piso ⇒ aparcado, NO encarnado.
        assert_eq!(g.parked.len(), 1, "sin piso el cliente queda aparcado");
        assert!(
            !g.incarnated.values().any(|i| i.card.label == "cliente-gui"),
            "el cliente no debe estar vivo todavía"
        );
        assert!(g.drain_refloorable().is_empty(), "sigue sin piso");

        // Aparece el piso: el compositor lo provee.
        let mut compositor = EntityCard::new("mirada");
        compositor.provides = [piso()].into_iter().collect();
        g.authorize_and_spawn(compositor, seed_id).await.unwrap();

        // El cliente ya es re-floorable.
        let ready = g.drain_refloorable();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].label, "cliente-gui");
        assert!(g.parked.is_empty(), "ya no queda nadie esperando piso");
    }

    #[tokio::test]
    async fn park_dedup_por_label() {
        let mut g = EnteGraph::new(seed_con_spawn());
        let seed_id = g.seed_id();
        let mk = || {
            let mut c = EntityCard::new("gui");
            c.requires = [piso()].into_iter().collect();
            c.supervision = Supervision::Restart {
                initial: Duration::from_millis(10),
                max: Duration::from_secs(1),
            };
            c
        };
        // Dos intentos del mismo label (dos ciclos de muerte/respawn sin piso).
        g.authorize_and_spawn(mk(), seed_id).await.unwrap();
        g.authorize_and_spawn(mk(), seed_id).await.unwrap();
        assert_eq!(g.parked.len(), 1, "un solo aparcado por label");
    }

    #[tokio::test]
    async fn readiness_dinamica_revive_al_aparcado() {
        // El compositor arranca pero anuncia su piso DESPUÉS (cuando su socket
        // ya escucha) vía capability dinámica — no estática. El cliente aparcado
        // debe volverse re-floorable en cuanto se anuncia.
        let mut g = EnteGraph::new(seed_con_spawn());
        let seed_id = g.seed_id();

        let mut client = EntityCard::new("cliente-gui");
        client.requires = [piso()].into_iter().collect();
        client.supervision = Supervision::Restart {
            initial: Duration::from_millis(10),
            max: Duration::from_secs(1),
        };
        g.authorize_and_spawn(client, seed_id).await.unwrap();
        assert_eq!(g.parked.len(), 1);

        // mirada vive pero todavía NO anunció el piso ⇒ nada re-floorea.
        let mut compositor = EntityCard::new("mirada");
        // (sin provides estáticos del piso)
        let comp_id = compositor.id;
        compositor.provides = [Capability::Journal].into_iter().collect();
        g.authorize_and_spawn(compositor, seed_id).await.unwrap();
        assert!(g.drain_refloorable().is_empty(), "aún no anunció readiness");

        // mirada anuncia readiness (UpdateCapabilities) ⇒ el cliente revive.
        g.register_dynamic_cap(comp_id, piso());
        let ready = g.drain_refloorable();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].label, "cliente-gui");
    }

    #[tokio::test]
    async fn oneshot_sin_piso_no_se_aparca() {
        let mut g = EnteGraph::new(seed_con_spawn());
        let seed_id = g.seed_id();
        let mut once = EntityCard::new("tarea");
        once.requires = [piso()].into_iter().collect();
        once.supervision = Supervision::OneShot;
        g.authorize_and_spawn(once, seed_id).await.unwrap();
        assert!(g.parked.is_empty(), "OneShot no persiste: se descarta, no aparca");
    }
}
