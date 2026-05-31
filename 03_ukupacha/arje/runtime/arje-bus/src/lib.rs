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
}

/// Estado de vida de un Ente, tal como lo conoce arje-zero.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Liveness {
    /// Vivo en el grafo. `pid` None para Entes Virtual/Wasm (sin proceso).
    Running { pid: Option<i32> },
    /// No está en el grafo: murió o nunca existió.
    Gone,
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
        }
    }
}
