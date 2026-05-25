//! `shuma-protocol` — wire daemon ↔ cliente (cli/gui).
//!
//! Framing: u32 BE length-prefix + payload postcard. Mismo patrón que
//! `ente-bus`/`brahman-handshake` para que clientes existentes compartan
//! reader/writer helpers si quieren.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use shuma_card::{PipelineSpec, WorkspaceId, WorkspaceSpec};
use std::path::PathBuf;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use ulid::Ulid;

pub const DEFAULT_SOCK_NAME: &str = "shuma.sock";
pub const MAX_FRAME: usize = 1 << 20;

fn default_grace_ms() -> u64 {
    1000
}

// =====================================================================
// Mensajes
// =====================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// Health-check.
    Ping,

    /// Health endpoint estructurado: versión + uptime + counts.
    Health,

    /// Crear un workspace nuevo.
    WorkspaceCreate { spec: WorkspaceSpec },

    /// Listar todos los workspaces vivos.
    WorkspaceList,

    /// Detener un workspace y reapear sus comandos. `grace_ms`: tiempo
    /// que se espera tras SIGTERM antes de SIGKILL. 0 = SIGKILL inmediato.
    WorkspaceStop {
        id: WorkspaceId,
        #[serde(default = "default_grace_ms")]
        grace_ms: u64,
    },

    /// Ejecutar un comando one-shot dentro de un workspace existente.
    Run {
        workspace: WorkspaceId,
        exec: String,
        argv: Vec<String>,
        envp: Vec<(String, String)>,
        /// Si `true` y el comando muere con exit_status != 0, el reaper
        /// lo relaunch con backoff exponencial.
        #[serde(default)]
        restart_on_failure: bool,
    },

    /// Lanzar un Pipeline completo dentro de un workspace.
    PipelineRun {
        spec: PipelineSpec,
        /// Si `true`, el daemon interpone un tap entre productor y
        /// consumidor de cada FlowEdge, sampleando los primeros bytes
        /// y discerniendo el TypeRef.
        tap: bool,
        /// Variables para sustitución `${KEY}` en strings del spec
        /// antes de spawn (templating).
        #[serde(default)]
        vars: std::collections::BTreeMap<String, String>,
    },

    /// Discernir un buffer ad-hoc (sin workspace). Útil para `shuma discern <file>`.
    Discern { sample: Vec<u8>, hint_path: Option<PathBuf> },

    /// Capacidades runtime del kernel/proceso del daemon.
    Capabilities,

    /// Listar comandos vivos+pasados de un workspace.
    CommandList { workspace: shuma_card::WorkspaceId },

    /// Tail del log capturado para un comando.
    CommandLogs {
        workspace: shuma_card::WorkspaceId,
        command: Ulid,
        tail_bytes: usize,
        /// "stdout" | "stderr" | "both" (default "both" si vacío).
        stream: String,
    },

    /// Guardar (o reemplazar) un PipelineSpec bajo un nombre.
    PipelineSave { name: String, spec: PipelineSpec },

    /// Listar nombres de pipelines guardados.
    PipelineSavedList,

    /// Eliminar un pipeline guardado.
    PipelineDrop { name: String },

    /// Ejecutar un pipeline guardado.
    PipelineRunSaved {
        name: String,
        tap: bool,
        #[serde(default)]
        vars: std::collections::BTreeMap<String, String>,
    },

    /// Resource accounting de un workspace.
    WorkspaceStats { workspace: shuma_card::WorkspaceId },

    /// Reporte de quotas (rlimits declarados vs uso actual).
    WorkspaceQuota { workspace: shuma_card::WorkspaceId },

    /// History de samples del workspace (server-side). Sobrevive
    /// restart del shell. `tail`: cantidad de samples desde el final
    /// (0 = todo).
    WorkspaceStatsHistory {
        workspace: shuma_card::WorkspaceId,
        tail: usize,
    },

    /// Resumen completo de un workspace: stats + quota + commands +
    /// flow sockets en una sola roundtrip. Reduce N×4 requests del
    /// shell a N×1.
    WorkspaceFullSummary { workspace: shuma_card::WorkspaceId },

    /// Detener selectivamente los comandos de un pipeline (no el workspace
    /// entero). `grace_ms`: SIGTERM → wait → SIGKILL.
    PipelineStop {
        pipeline: Ulid,
        #[serde(default = "default_grace_ms")]
        grace_ms: u64,
    },

    /// Listar pipelines activos con sus flow channels (data plane).
    FlowList,

    /// Throughput por flow socket: bytes_total + bytes_per_sec.
    FlowThroughput,

    /// Cerrar el data plane de un pipeline (drop sockets + canales).
    FlowDrop { pipeline: Ulid },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    Pong,

    Health {
        version: String,
        uptime_ms: u64,
        alive_workspaces: u32,
        alive_commands: u32,
        alive_pipelines: u32,
        active_flows: u32,
        dirty: bool,
    },

    WorkspaceCreated {
        id: WorkspaceId,
        warnings: Vec<String>,
    },

    WorkspaceList {
        items: Vec<WorkspaceSummary>,
    },

    WorkspaceStopped {
        id: WorkspaceId,
        reaped: u32,
    },

    RunStarted {
        workspace: WorkspaceId,
        command_id: Ulid,
        pid: i32,
    },

    PipelineStarted {
        pipeline: Ulid,
        command_pids: Vec<(String, i32)>,
        /// Discernments por edge cuando tap=true. Vacío sin tap.
        edges: Vec<EdgeDiscernmentInfo>,
    },

    Discernment {
        ty: String,
        confidence: f32,
        mime: Option<String>,
        lens: Option<String>,
    },

    Capabilities {
        kernel_version: (u32, u32, u32),
        user_ns: String,
        cgroup_v2: String,
        cgroup_delegated: bool,
        has_cap_sys_admin: bool,
    },

    CommandList {
        items: Vec<CommandInfo>,
    },

    CommandLogs {
        bytes: Vec<u8>,
    },

    PipelineSaved {
        name: String,
    },

    PipelineSavedList {
        names: Vec<String>,
    },

    PipelineDropped {
        name: String,
        existed: bool,
    },

    PipelineStopped {
        pipeline: Ulid,
        reaped: u32,
    },

    WorkspaceStats {
        info: WorkspaceStatsInfo,
    },

    WorkspaceQuota {
        info: QuotaReportInfo,
    },

    WorkspaceStatsHistory {
        samples: Vec<WorkspaceStatsInfo>,
    },

    WorkspaceFullSummary {
        stats: WorkspaceStatsInfo,
        quota: QuotaReportInfo,
        commands: Vec<CommandInfo>,
        flow_sockets: Vec<PathBuf>,
    },

    FlowList {
        items: Vec<FlowInfo>,
    },

    FlowThroughput {
        items: Vec<FlowThroughputInfo>,
    },

    FlowDropped {
        pipeline: Ulid,
        existed: bool,
    },

    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaReportInfo {
    pub mem_limit: Option<u64>,
    pub nproc_limit: Option<u32>,
    pub breaches: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceStatsInfo {
    pub commands_alive: u32,
    pub commands_total: u32,
    pub rss_bytes: Option<u64>,
    #[serde(default)]
    pub rss_peak_bytes: Option<u64>,
    pub cpu_usec: Option<u64>,
    #[serde(default)]
    pub cpu_percent: Option<f32>,
    #[serde(default = "default_cpu_cores")]
    pub cpu_cores: u32,
    pub source: String,
    pub uptime_ms: u64,
}

fn default_cpu_cores() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowThroughputInfo {
    pub socket: PathBuf,
    pub bytes_total: u64,
    pub bytes_per_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowInfo {
    pub pipeline: Ulid,
    pub sockets: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandInfo {
    pub id: Ulid,
    pub label: String,
    pub pid: i32,
    pub alive: bool,
    pub exit_status: Option<i32>,
    pub log_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeDiscernmentInfo {
    pub from_label: String,
    pub from_output: String,
    pub to_label: String,
    pub to_input: String,
    /// `Some(ty)` si el discerner detectó algo. `None` si no hubo data
    /// suficiente o no matcheó ningún discerner.
    pub ty: Option<String>,
    pub mime: Option<String>,
    pub lens: Option<String>,
    pub confidence: f32,
    /// Path del Unix socket donde otros módulos pueden suscribirse a los
    /// bytes replicados de este edge (data plane). `None` si tap=false.
    pub flow_socket: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSummary {
    pub id: WorkspaceId,
    pub label: String,
    pub commands: u32,
    pub uptime_ms: u64,
}

// =====================================================================
// Errores
// =====================================================================

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("frame oversize: {0} bytes (max {MAX_FRAME})")]
    FrameOversize(usize),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("postcard: {0}")]
    Postcard(#[from] postcard::Error),
    #[error("connection closed")]
    Closed,
}

// =====================================================================
// Framing helpers
// =====================================================================

pub async fn write_frame<T: Serialize>(stream: &mut UnixStream, msg: &T) -> Result<(), ProtocolError> {
    let bytes = postcard::to_allocvec(msg)?;
    if bytes.len() > MAX_FRAME {
        return Err(ProtocolError::FrameOversize(bytes.len()));
    }
    let len = (bytes.len() as u32).to_be_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

pub async fn read_frame<T: for<'de> Deserialize<'de>>(
    stream: &mut UnixStream,
) -> Result<T, ProtocolError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            ProtocolError::Closed
        } else {
            ProtocolError::Io(e)
        }
    })?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(ProtocolError::FrameOversize(len));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(postcard::from_bytes(&buf)?)
}

/// Path canónico del socket del daemon: `$XDG_RUNTIME_DIR/shuma.sock`,
/// fallback `/run/user/$UID/shuma.sock`, fallback `/tmp/shuma-$UID.sock`.
pub fn default_socket_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(xdg).join(DEFAULT_SOCK_NAME);
    }
    let uid = nix::unistd::getuid().as_raw();
    let p = PathBuf::from(format!("/run/user/{uid}"));
    if p.exists() {
        return p.join(DEFAULT_SOCK_NAME);
    }
    PathBuf::from(format!("/tmp/shuma-{uid}.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_roundtrip() {
        let bytes = postcard::to_allocvec(&Request::Ping).unwrap();
        let back: Request = postcard::from_bytes(&bytes).unwrap();
        assert!(matches!(back, Request::Ping));
    }

    #[test]
    fn workspace_create_roundtrip() {
        let req = Request::WorkspaceCreate {
            spec: WorkspaceSpec {
                label: "demo".into(),
                soma: Default::default(),
                permissions: Default::default(),
                ttl: None,
                flow_dirs: vec![],
                on_exit: shuma_card::ExitPolicy::Reap,
                quota_enforce: Default::default(),
            },
        };
        let bytes = postcard::to_allocvec(&req).unwrap();
        let back: Request = postcard::from_bytes(&bytes).unwrap();
        match back {
            Request::WorkspaceCreate { spec } => assert_eq!(spec.label, "demo"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn default_socket_path_uses_runtime_dir() {
        let p = default_socket_path();
        assert!(p.to_string_lossy().ends_with("shuma.sock"));
    }
}
