//! `shuma-core` — runtime in-memory de Workspaces y comandos.
//!
//! Mantiene un estado tokio-friendly (Mutex sobre HashMap) con:
//! - Workspaces vivos (id → state).
//! - PIDs de comandos lanzados, indexados por workspace.
//! - Reaping cooperativo: `reap_dead()` cosecha hijos terminados.

// `pipeline` necesita `unsafe` puntual para `libc::close` y construir
// `OwnedFd` desde fds que armamos con `pipe2(2)`. El resto del crate
// permanece safe — el cargo lint `unsafe_code` queda permitido sólo en
// el módulo concreto.
#![deny(unsafe_op_in_unsafe_fn)]

pub mod flow_channel;
pub mod logbuf;
pub mod persist;
pub mod pipeline;
pub mod stats;

use card_core::{Card, Payload, Supervision};
use arje_incarnate::{Incarnator, IncarnatorConfig};
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use shuma_card::{CommandRef, PipelineSpec, WorkspaceId, WorkspaceSpec};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::{info, warn};
use ulid::Ulid;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("workspace {0} not found")]
    WorkspaceNotFound(WorkspaceId),
    #[error("compile: {0}")]
    Compile(#[from] shuma_card::CompileError),
    #[error("incarnate: {0}")]
    Incarnate(#[from] arje_incarnate::IncarnateError),
}

#[derive(Debug)]
pub struct WorkspaceState {
    pub id: WorkspaceId,
    pub spec: WorkspaceSpec,
    pub root_card: Card,
    pub commands: HashMap<Ulid, CommandState>,
    pub started: Instant,
    /// Última muestra de `(wall_instant, cpu_usec)` usada para calcular
    /// `cpu_percent` en la próxima medición. None hasta el primer measure.
    pub last_cpu_sample: Option<(Instant, u64)>,
    /// Ring buffer de samples recientes para sparklines. Se popula cada
    /// vez que `workspace_stats` se llama (típicamente desde el shell).
    /// Cap 64 samples = ~2 minutos a 2s/sample.
    pub stats_history: std::collections::VecDeque<stats::WorkspaceStats>,
}

const STATS_HISTORY_CAP: usize = 64;

#[derive(Debug, Clone)]
pub struct CommandState {
    pub id: Ulid,
    pub label: String,
    pub pid: Pid,
    pub alive: bool,
    pub exit_status: Option<i32>,
    /// Ring buffer del stdout. `None` para comandos sin captura.
    pub stdout: Option<logbuf::LogBuf>,
    /// Ring buffer del stderr. Separado de `stdout` para que el CLI
    /// pueda filtrarlos. `None` para comandos sin captura.
    pub stderr: Option<logbuf::LogBuf>,
    /// Si el comando fue lanzado como parte de un Pipeline, su ULID.
    pub pipeline_id: Option<Ulid>,
}

/// Stream a leer en `get_command_logs`. `Both` concatena stderr-después-stdout
/// para una vista combinada (orden temporal aproximado).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
    Both,
}

pub struct WorkspaceManager {
    inner: Arc<Mutex<Inner>>,
    incarnator: Arc<Incarnator>,
    /// True si hubo alguna mutación desde el último `save_snapshot`.
    /// `save_snapshot` skip si false (snapshot incremental — evita
    /// re-serialize cuando nada cambió, ej. SIGTERM tras un período idle).
    dirty: std::sync::atomic::AtomicBool,
}

struct Inner {
    workspaces: HashMap<WorkspaceId, WorkspaceState>,
    /// Definiciones nombradas de pipelines persistidas. NO es lo mismo
    /// que "pipelines vivos" — son specs guardados para reusar con
    /// `run-saved`. Sobreviven restart vía snapshot.
    saved_pipelines: HashMap<String, PipelineSpec>,
    /// Flow channels vivos por pipeline. Se retienen hasta que el
    /// pipeline termine — cuando todos los hijos del pipeline murieron,
    /// el reaper los borra (futuro). v1: viven hasta `stop_pipeline_flows`
    /// explícito o hasta shutdown.
    pipeline_flows: HashMap<Ulid, Vec<crate::flow_channel::FlowChannel>>,
    /// Specs de comandos `run()` con `restart_on_failure=true`. Indexed
    /// por command_id. Cuando `reap_dead` detecta exit!=0, se relauncha
    /// con la misma spec (nuevo pid y nuevo command_id se asigna por
    /// el nuevo state pero el restart_spec sigue ligado al original).
    restart_specs: HashMap<Ulid, RestartSpec>,
    /// Supervisores de pipelines con `restart_on_failure`. Indexed por
    /// pipeline_id. Cuando `reap_dead` detecta que el pipeline tuvo
    /// algún command failure, agrega un entry a `pending_pipeline_restarts`.
    pipeline_supervisors: HashMap<Ulid, PipelineSupervisor>,
    /// Cola de pipelines pendientes de restart. El daemon la drena en
    /// cada loop del reaper, hace stop + run_pipeline.
    pending_pipeline_restarts: Vec<Ulid>,
}

#[derive(Debug, Clone)]
pub struct PipelineSupervisor {
    pub workspace: WorkspaceId,
    pub spec: PipelineSpec,
    pub tap: bool,
    pub restart_count: u32,
    /// Backoff actual (ms) — escala exponencialmente con cada restart.
    pub current_backoff_ms: u64,
}

#[derive(Debug, Clone)]
struct RestartSpec {
    workspace: WorkspaceId,
    exec: String,
    argv: Vec<String>,
    envp: Vec<(String, String)>,
    /// Backoff inicial (ms). Crece exponencialmente hasta max_backoff_ms.
    backoff_ms: u64,
    max_backoff_ms: u64,
    /// Cantidad de restarts ya ejecutados (para tracking).
    restart_count: u32,
}

#[derive(Debug, Clone)]
pub struct CommandSummary {
    pub id: Ulid,
    pub label: String,
    pub pid: i32,
}

#[derive(Debug, Clone, Default)]
pub struct HealthCounts {
    pub alive_workspaces: u32,
    pub alive_commands: u32,
    pub alive_pipelines: u32,
    pub active_flows: u32,
}

#[derive(Debug, Clone)]
pub struct CommandInfo {
    pub id: Ulid,
    pub label: String,
    pub pid: i32,
    pub alive: bool,
    pub exit_status: Option<i32>,
    pub log_bytes: u64,
}

/// Lee VmRSS (bytes) de `/proc/<pid>/status`. Helper local para
/// reap_dead que no necesita el full stats. Devuelve 0 si el proc no
/// existe o el campo no aparece.
fn read_proc_rss(pid: i32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:").map(str::trim))
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse::<u64>().ok())
        .map(|kb| kb * 1024)
}

fn spawn_log_drainer(read_fd: std::os::fd::RawFd, logs: logbuf::LogBuf) {
    // Marcar non-blocking + envolver en AsyncFd; igual patrón que el tap.
    // SAFETY: F_SETFL sobre fd válido.
    unsafe {
        let flags = libc::fcntl(read_fd, libc::F_GETFL, 0);
        if flags >= 0 {
            libc::fcntl(read_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
    tokio::spawn(async move {
        // SAFETY: ownership del fd transferido al drainer task.
        let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd_compat(read_fd) };
        let afd = match tokio::io::unix::AsyncFd::with_interest(owned, tokio::io::Interest::READABLE) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(?e, "log drainer AsyncFd failed");
                return;
            }
        };
        let mut buf = [0u8; 4096];
        loop {
            let mut guard = match afd.readable().await {
                Ok(g) => g,
                Err(_) => break,
            };
            use std::os::fd::AsRawFd;
            let fd = afd.as_raw_fd();
            // SAFETY: read sobre fd válido.
            let r = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if r > 0 {
                logs.append(&buf[..r as usize]);
                continue;
            }
            if r == 0 {
                break; // EOF
            }
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                guard.clear_ready();
                continue;
            }
            tracing::warn!(?err, "log drainer read err");
            break;
        }
    });
}

trait OwnedFdFromRawCompat: Sized {
    unsafe fn from_raw_fd_compat(fd: std::os::fd::RawFd) -> Self;
}

impl OwnedFdFromRawCompat for std::os::fd::OwnedFd {
    unsafe fn from_raw_fd_compat(fd: std::os::fd::RawFd) -> Self {
        use std::os::fd::FromRawFd;
        // SAFETY: el caller transfiere ownership de fd a OwnedFd.
        unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) }
    }
}


#[derive(Debug, Clone)]
pub struct WorkspaceSnapshot {
    pub id: WorkspaceId,
    pub label: String,
    pub commands: u32,
    pub uptime_ms: u64,
}

fn short_ulid(u: &Ulid) -> String {
    let s = u.to_string();
    s[s.len() - 6..].to_string()
}

// `impl WorkspaceManager` partido por dominio (regla dura #1, 1517 LOC):
mod pipelines;
mod runtime;
mod workspaces;

#[cfg(test)]
mod tests;
