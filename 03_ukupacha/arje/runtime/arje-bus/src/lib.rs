//! ente-bus: bus de capacidades interno del fractal.
//!
//! Wire format: Unix SOCK_STREAM con framing length-prefijo (u32 BE) + payload
//! postcard. Bidireccional pero por ahora request-response síncrono.
//!
//! Identidad: cada Ente hijo recibe `ENTE_BUS_SOCK` y `ENTE_ID` en su entorno.
//! El cliente lee ambos vía `BusClient::from_env`.

use arje_card::Capability;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::str::FromStr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use ulid::Ulid;

pub const ENV_BUS_SOCK: &str = "ENTE_BUS_SOCK";
pub const ENV_ENTE_ID: &str = "ENTE_ID";
pub const MAX_FRAME: usize = 1 << 20; // 1 MiB — protección contra OOM

/// Interface UUID para decisiones de policy. Un Ente independiente
/// (separado de polkit-compat) se anuncia como proveedor de
/// `Capability::Endpoint { interface: POLKIT_DECISION_IFACE, version: 1 }`
/// para arbitrar autorizaciones. Recibe blob:
/// `pid_be | uid_be | action_id_utf8` → responde 1 byte: 1=allow, 0=deny.
pub const POLKIT_DECISION_IFACE: arje_card::InterfaceId =
    arje_card::InterfaceId([0xb0; 16]);

/// Interface UUID auto-anunciado por compat-polkit. Diferente al de
/// decisión para evitar recursión (polkit-compat invoca DECISION pero
/// no es proveedor de DECISION; se anuncia como SERVICE).
pub const POLKIT_SERVICE_IFACE: arje_card::InterfaceId =
    arje_card::InterfaceId([0xa4; 16]);

/// Interface UUID por la que el cerebro entrega notificaciones dirigidas a
/// un Ente concreto. El blob es UTF-8 del mensaje. Provee canal sin protocolo
/// para que reglas del fractal puedan poke a Entes específicos sin un Endpoint
/// por dominio. El proveedor decide cómo reaccionar (log, hint, etc).
pub const BRAIN_NOTIFY_IFACE: arje_card::InterfaceId =
    arje_card::InterfaceId([0xbe; 16]);

/// Credenciales del peer extraídas vía SO_PEERCRED al accept del bus.
/// Imposibles de falsear desde el cliente — el kernel las inyecta.
/// Definidas aquí (no en ente-zero) porque conceptualmente son atributo
/// del protocolo del bus, no del init.
#[derive(Debug, Clone, Copy)]
pub struct PeerCreds {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusMessage {
    pub from: Option<Ulid>,
    pub seq: u64,
    pub payload: BusPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BusPayload {
    Request(BusRequest),
    Response(BusResponse),
    /// Notificación difundida por arje-zero hacia las conexiones suscritas
    /// (`BusRequest::Subscribe`). A diferencia de Request/Response, no lleva
    /// `seq` correlacionable ni espera contestación: es fire-and-forget
    /// server→cliente. Es el vocabulario de observabilidad de ciclo de vida
    /// que permite que un supervisor externo (p. ej. la capa de IA de hammer,
    /// `hammerd`) reaccione a un crash sin pollear `ListEntes`.
    Event(BusEvent),
}

/// Estado terminal de un Ente, en forma serializable para el wire.
/// Espejo de `arje-zero`'s `ExitStatus` sin depender de `nix::Signal`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum LifecycleStatus {
    /// Terminó con `exit(code)`. `0` = limpio; cualquier otro = anómalo.
    Exited(i32),
    /// Murió por señal POSIX (número crudo, p. ej. 9 = SIGKILL, 11 = SIGSEGV).
    Killed(i32),
}

impl LifecycleStatus {
    /// `true` si la terminación fue anómala (exit≠0 o cualquier señal).
    pub fn is_crash(&self) -> bool {
        !matches!(self, LifecycleStatus::Exited(0))
    }
}

/// Evento de ciclo de vida difundido a los suscriptores del bus. Es la
/// **fuente real** del `CRASHED` que la capa de IA esperaba (hammer roadmap
/// Fase 5 / B.2): arje supervisa, detecta la muerte en `on_death`, y la
/// difunde aquí; el suscriptor la traduce a su propio protocolo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BusEvent {
    /// Un Ente supervisado terminó de forma anómala (exit≠0 o señal).
    EnteCrashed { id: Ulid, label: String, status: LifecycleStatus },
    /// arje programó un reinicio del Ente tras el backoff. `delay_ms` es la
    /// espera antes del próximo intento.
    EnteRestarting { id: Ulid, label: String, delay_ms: u64 },
    /// Un Ente terminó de forma limpia (exit 0) y no será reiniciado.
    EnteExited { id: Ulid, label: String },
    /// Un Ente `Restart` quedó APARCADO porque su "piso" (una capability de la
    /// que depende) no tiene proveedor — p. ej. el compositor cayó y se llevó a
    /// este cliente. Espera a que el proveedor reaparezca; no está vivo ni
    /// muerto-definitivo. El monitor lo muestra como "esperando piso".
    EnteParked { id: Ulid, label: String },
    /// El piso volvió y arje re-spawnea al Ente aparcado (en orden topológico).
    /// Cierra el ciclo de un `EnteParked` previo.
    EnteRefloored { id: Ulid, label: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BusRequest {
    /// Saludo. El Ente anuncia que está vivo y declara sus capacidades.
    /// El Init usa esto para saber que el child arrancó correctamente,
    /// independientemente de su exit code.
    Announce { capabilities: Vec<Capability> },

    /// Listar Entes vivos. Útil para debugging y para Entes-supervisor.
    ListEntes,

    /// Control de estado del fractal. Traducido desde D-Bus por compat-logind.
    PowerOff { interactive: bool },
    Reboot { interactive: bool },
    Suspend { interactive: bool },
    Hibernate { interactive: bool },

    /// Invocación genérica de capacidad. `cap` debe estar provista por algún
    /// Ente del grafo; el blob es el argumento opaco que el proveedor parsea.
    Invoke { cap: Capability, blob: Vec<u8> },

    /// Actualización dinámica del set de capacidades del Ente que llama.
    /// Sólo aplicable al `from_authenticated` — un Ente sólo puede modificar
    /// sus propias caps. La Card original (immutable) no se toca; la mutación
    /// va al `dynamic_provides` del Incarnated.
    UpdateCapabilities {
        adds: Vec<Capability>,
        removes: Vec<Capability>,
    },

    /// Enviar una señal POSIX a un Ente del fractal. Requiere identidad
    /// autenticada del caller — el shim systemd1 lo usa para implementar
    /// `KillUnit`. arje-zero arbitra: lookup del Ulid, captura del PID y
    /// `kill(pid, signal)`. Sólo Entes con `Payload::Native|Legacy` son
    /// matables (los Virtual y los Wasm responden NotApplicable).
    KillEnte { target: Ulid, signal: i32 },

    /// Pide a arje-zero que cargue una Card por nombre desde el store en
    /// disco (`/etc/arje/cards.d/{name}.json`, override con env
    /// `ARJE_CARDS_DIR`) y la encarne. Requiere identidad autenticada —
    /// el caller queda registrado en logs; la Card spawnea con la
    /// Semilla como `requester` para satisfacer Capability::Spawn sin
    /// distribuirla a cada shim de compat.
    SpawnCardFromDisk { name: String },

    /// Inverso de `SpawnCardFromDisk`: baja los Entes vivos cuyos labels
    /// declara la Card `{name}.json` del store (su raíz si es un único Ente,
    /// o los de su `genesis` si es un bundle `Virtual`). A diferencia de
    /// `KillEnte`, **no reinicia**: marca cada miembro para detención y le
    /// manda SIGTERM, de modo que su supervisor `Restart` no lo revive. Es
    /// el teardown de una sesión (p. ej. al volver de gnome a mirada).
    /// Requiere identidad autenticada. Idempotente: miembros ya muertos se
    /// ignoran.
    StopCardFromDisk { name: String },

    /// Encarna una Card **transmitida por el wire** (no del store en disco).
    /// Es el transporte de `sandokan_core::Engine::run` con una Card arbitraria
    /// (ver `shared/sandokan/SDD.md` §5). Modelo de confianza (a diferencia de
    /// `SpawnCardFromDisk`, que spawnea con la Semilla): el caller debe tener
    /// `Capability::Spawn` y la Card se encarna con el **caller como requester**
    /// —hereda sus capacidades, no las de la Semilla—, así que no hay escalada
    /// de privilegios. La Card viaja como [`WireCard`](arje_card::WireCard)
    /// (proyección postcard-friendly; las `extensions` locales no cruzan).
    RunCard { card: arje_card::WireCard },

    /// Estado de vida de un Ente. arje-zero sólo distingue "vivo" (en el
    /// grafo) de "ido" (muerto/inexistente): no retiene exit codes tras la
    /// muerte. Observabilidad — anónimo, como `ListEntes`. Es el vocabulario
    /// que faltaba para que arje-bus cubra el contrato `sandokan-core::Engine`
    /// (ver `shared/sandokan/SDD.md` §5 Fase 2).
    EnteStatus { target: Ulid },

    /// Telemetría puntual de un Ente: arje-zero lee `/proc/<pid>` y devuelve
    /// memoria residente + nº de hilos. Anónimo. `Error` si el Ente no vive o
    /// no tiene PID (Virtual/Wasm).
    EnteTelemetry { target: Ulid },

    /// Suscribe esta conexión al stream de eventos de ciclo de vida
    /// (`BusEvent`). Tras un `Ok`, arje-zero empuja un `BusPayload::Event`
    /// por cada muerte/crash/reinicio de Ente. Anónimo (observabilidad, como
    /// `ListEntes`): no requiere identidad autenticada. La conexión deja de
    /// servir para request-response — pasa a ser un canal de sólo-eventos.
    Subscribe,

    // NOTA DE WIRE: postcard numera las variantes por **posición**. Agregar
    // siempre al FINAL — insertar en el medio corre los discriminantes de las
    // de abajo y rompe a consumidores que los hardcodean (`hammerd::arje_link`
    // pinea el byte de `Subscribe`; hay un test que lo delata).
    /// Reescribe `cpu.weight` de un cgroup **ya existente** (priorizar o
    /// deprioritizar en caliente, sin reencarnar). `cgroup_path` se direcciona
    /// como `CgroupSpec.path` (relativo → bajo el cgroup actual); el peso es
    /// jerárquico, así que gobierna todo el subárbol —el slice de un contexto
    /// `pacha`— de una sola escritura. Transporte de
    /// `sandokan_core::Engine::set_cpu_weight` (SDD §8 capa 1). Requiere
    /// identidad autenticada; queda en la cadena de auditoría.
    SetCpuWeight { cgroup_path: String, weight: u32 },

    /// Congela (`true`) o descongela (`false`) un cgroup vía el freezer v2
    /// (`cgroup.freeze`). Jerárquico: gobierna todo el subárbol → SIGSTOP de
    /// grupo conservando la RAM. Transporte de `sandokan_core::Engine::freeze`
    /// (SDD §8 capa 1). Requiere identidad autenticada; auditado.
    Freeze { cgroup_path: String, frozen: bool },
}

/// Estado de vida de un Ente, tal como lo conoce arje-zero.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Liveness {
    /// Vivo en el grafo. `pid` None para Entes Virtual/Wasm (sin proceso).
    Running { pid: Option<i32> },
    /// No está en el grafo: murió o nunca existió.
    Gone,
    /// APARCADO: no corre porque su piso (una capability de la que depende) no
    /// tiene proveedor; el Init lo re-erige cuando vuelva. `reason` = qué falta.
    Parked { reason: String },
}

/// Muestra puntual de recursos de un Ente (leída de `/proc/<pid>`) + su
/// conteo de restarts (que sólo conoce el supervisor, no `/proc`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSample {
    /// Memoria residente en bytes (RSS).
    pub mem_bytes: u64,
    /// Número de hilos del proceso.
    pub nproc: u32,
    /// Restarts acumulados que el Init le aplicó (0 si OneShot/Delegate o
    /// nunca reinició).
    pub restarts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BusResponse {
    Ok,
    Error(String),
    Entes(Vec<EnteInfo>),
    Invoked { result: Vec<u8> },
    Status(Liveness),
    Telemetry(ResourceSample),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnteInfo {
    pub id: Ulid,
    pub label: String,
    pub provides: Vec<Capability>,
    pub pid: Option<i32>,
}

pub async fn write_frame<W: AsyncWriteExt + Unpin>(w: &mut W, msg: &BusMessage) -> anyhow::Result<()> {
    let bytes = postcard::to_stdvec(msg)?;
    if bytes.len() > MAX_FRAME {
        anyhow::bail!("frame too large: {} > {}", bytes.len(), MAX_FRAME);
    }
    w.write_u32(bytes.len() as u32).await?;
    w.write_all(&bytes).await?;
    Ok(())
}

pub async fn read_frame<R: AsyncReadExt + Unpin>(r: &mut R) -> anyhow::Result<BusMessage> {
    let len = r.read_u32().await? as usize;
    if len > MAX_FRAME {
        anyhow::bail!("frame oversize: {len}");
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(postcard::from_bytes(&buf)?)
}

pub struct BusClient {
    stream: UnixStream,
    seq: u64,
    self_id: Option<Ulid>,
}

/// Trait que un Ente proveedor implementa para servir invokes que el bus le
/// forwarda. Sync por simplicidad — un handler async se cubre con
/// `tokio::task::block_in_place` o un canal hacia un task externo.
pub trait InvokeHandler {
    fn handle(&mut self, cap: arje_card::Capability, blob: Vec<u8>) -> BusResponse;
}

/// Conexión long-lived para Entes que proveen capacidades. A diferencia de
/// `BusClient` (request-response y desconecta), `BusServer`:
///   1. Anuncia su identidad al bus
///   2. Mantiene la conexión abierta
///   3. Atiende invokes forwardeados por el bus en bucle
pub struct BusServer {
    stream: UnixStream,
    self_id: Ulid,
}

impl BusServer {
    pub async fn from_env() -> anyhow::Result<Self> {
        let path = std::env::var(ENV_BUS_SOCK)
            .map_err(|_| anyhow::anyhow!("{} no definido", ENV_BUS_SOCK))?;
        let id_s = std::env::var(ENV_ENTE_ID)
            .map_err(|_| anyhow::anyhow!("{} no definido", ENV_ENTE_ID))?;
        let self_id = Ulid::from_str(&id_s)
            .map_err(|_| anyhow::anyhow!("{} no es un Ulid válido: {id_s}", ENV_ENTE_ID))?;
        let stream = UnixStream::connect(&path).await?;
        Ok(Self { stream, self_id })
    }

    pub async fn announce(&mut self, capabilities: Vec<arje_card::Capability>) -> anyhow::Result<()> {
        let req = BusMessage {
            from: Some(self.self_id),
            seq: 1,
            payload: BusPayload::Request(BusRequest::Announce { capabilities }),
        };
        write_frame(&mut self.stream, &req).await?;
        let resp = read_frame(&mut self.stream).await?;
        match resp.payload {
            BusPayload::Response(BusResponse::Ok) => Ok(()),
            BusPayload::Response(other) => anyhow::bail!("Announce rechazado: {other:?}"),
            BusPayload::Request(_) => anyhow::bail!("expected Response, got Request"),
            BusPayload::Event(_) => anyhow::bail!("expected Response, got Event"),
        }
    }

    /// Bucle principal del proveedor. Atiende invokes hasta que la conexión
    /// se cierra (el bus muere o el Ente recibe SIGTERM y nosotros dropeamos).
    pub async fn serve<H: InvokeHandler>(mut self, mut handler: H) -> anyhow::Result<()> {
        loop {
            let msg = read_frame(&mut self.stream).await?;
            let resp = match msg.payload {
                BusPayload::Request(BusRequest::Invoke { cap, blob }) => {
                    handler.handle(cap, blob)
                }
                BusPayload::Request(other) => {
                    BusResponse::Error(format!("BusServer no maneja {other:?}"))
                }
                BusPayload::Response(_) => continue,
                BusPayload::Event(_) => continue,
            };
            let out = BusMessage {
                from: Some(self.self_id),
                seq: msg.seq,
                payload: BusPayload::Response(resp),
            };
            write_frame(&mut self.stream, &out).await?;
        }
    }
}

impl BusClient {
    pub async fn connect(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(path).await?;
        let self_id = std::env::var(ENV_ENTE_ID)
            .ok()
            .and_then(|s| Ulid::from_str(&s).ok());
        Ok(Self { stream, seq: 0, self_id })
    }

    pub async fn from_env() -> anyhow::Result<Self> {
        let path = std::env::var(ENV_BUS_SOCK)
            .map_err(|_| anyhow::anyhow!("{} no definido", ENV_BUS_SOCK))?;
        Self::connect(&path).await
    }

    pub async fn call(&mut self, req: BusRequest) -> anyhow::Result<BusResponse> {
        self.seq = self.seq.wrapping_add(1);
        let msg = BusMessage {
            from: self.self_id,
            seq: self.seq,
            payload: BusPayload::Request(req),
        };
        write_frame(&mut self.stream, &msg).await?;
        let resp = read_frame(&mut self.stream).await?;
        match resp.payload {
            BusPayload::Response(r) => Ok(r),
            BusPayload::Request(_) => anyhow::bail!("expected response, got request"),
            BusPayload::Event(_) => anyhow::bail!("expected response, got event"),
        }
    }

    /// Suscribe esta conexión al stream de eventos de ciclo de vida. Tras un
    /// `Ok` del servidor, la conexión pasa a modo sólo-eventos: usá
    /// [`next_event`](Self::next_event) para drenarlos. No mezclar con `call`.
    pub async fn subscribe(&mut self) -> anyhow::Result<()> {
        match self.call(BusRequest::Subscribe).await? {
            BusResponse::Ok => Ok(()),
            other => anyhow::bail!("Subscribe rechazado: {other:?}"),
        }
    }

    /// Bloquea hasta el próximo `BusEvent` difundido por arje-zero. Ignora
    /// cualquier frame que no sea un evento (no debería llegar otra cosa por
    /// una conexión suscrita). Devuelve error si la conexión se cierra.
    pub async fn next_event(&mut self) -> anyhow::Result<BusEvent> {
        loop {
            let msg = read_frame(&mut self.stream).await?;
            match msg.payload {
                BusPayload::Event(ev) => return Ok(ev),
                _ => continue,
            }
        }
    }
}

#[cfg(test)]
mod contrato_wire {
    //! Contrato de wire con el **espejo** de hammer (`hammerd::arje_link`).
    //!
    //! hammer NO depende de este crate: relee el frame postcard con tipos espejo
    //! y discriminantes hardcodeados (B.2 del PLAN-ATESTACION-Y-HAMMER). Si acá se
    //! reordena un enum del wire, el espejo se rompe **en silencio** (el init deja
    //! de entregar crashes). Estos asserts fijan los bytes exactos que el espejo
    //! asume; si fallan, hay que actualizar `hammerd::arje_link` en el repo hammer.
    use super::*;

    #[test]
    fn subscribe_serializa_a_los_bytes_que_espera_hammer() {
        // hammerd::arje_link::subscribe_body(1) hardcodea [0x00,0x01,0x00,0x0D]:
        // from=None, seq=1, BusPayload::Request(0), BusRequest::Subscribe(13).
        let m = BusMessage {
            from: None,
            seq: 1,
            payload: BusPayload::Request(BusRequest::Subscribe),
        };
        assert_eq!(
            postcard::to_stdvec(&m).unwrap(),
            vec![0x00, 0x01, 0x00, 0x0D],
            "el frame Subscribe cambió — actualizá hammerd::arje_link::subscribe_body \
             (¿se reordenó BusRequest o BusPayload?)",
        );
    }

    #[test]
    fn ente_crashed_conserva_los_discriminantes_del_espejo() {
        let id = Ulid::from_string("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
        let m = BusMessage {
            from: None,
            seq: 0,
            payload: BusPayload::Event(BusEvent::EnteCrashed {
                id,
                label: "d".into(),
                status: LifecycleStatus::Killed(11),
            }),
        };
        let bytes = postcard::to_stdvec(&m).unwrap();
        // [from=None=0x00][seq=0=0x00][payload=Event=0x02][event=EnteCrashed=0x00]…
        assert_eq!(
            &bytes[0..4],
            &[0x00, 0x00, 0x02, 0x00],
            "el layout de BusPayload::Event/BusEvent::EnteCrashed cambió — \
             actualizá el espejo de hammerd::arje_link",
        );
        // …seguido del Ulid serializado como string de 26 chars (el espejo lo asume).
        assert_eq!(
            bytes[4], 26,
            "Ulid dejó de serializarse como string de 26 — rompe el espejo de hammer",
        );
    }
}
