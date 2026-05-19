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

use brahman_card::{Card, Payload, Supervision};
use ente_incarnate::{Incarnator, IncarnatorConfig};
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
    Incarnate(#[from] ente_incarnate::IncarnateError),
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

impl WorkspaceManager {
    pub fn new(cfg: IncarnatorConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                workspaces: HashMap::new(),
                saved_pipelines: HashMap::new(),
                pipeline_flows: HashMap::new(),
                restart_specs: HashMap::new(),
                pipeline_supervisors: HashMap::new(),
                pending_pipeline_restarts: Vec::new(),
            })),
            incarnator: Arc::new(Incarnator::new(cfg)),
            dirty: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Marca el manager como dirty. Cualquier mutación que afecta al
    /// snapshot debería llamar esto.
    #[inline]
    fn mark_dirty(&self) {
        self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// True si hubo cambios desde el último `save_snapshot`. Útil para
    /// chequeos cooperativos (ej. monitoring que pollea cada N).
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Registra un supervisor para un pipeline con `restart_on_failure=true`.
    /// El daemon llama esto tras `run_pipeline` para que `reap_dead` agregue
    /// el pipeline a la cola de restart cuando algún command falle.
    pub async fn register_pipeline_supervisor(
        &self,
        pipeline_id: Ulid,
        workspace: WorkspaceId,
        spec: PipelineSpec,
        tap: bool,
    ) {
        if !spec.restart_on_failure {
            return;
        }
        tracing::debug!(%pipeline_id, label = %spec.label, "pipeline supervisor registered");
        let mut g = self.inner.lock().await;
        let initial_backoff = spec.restart_backoff_ms.max(50);
        g.pipeline_supervisors.insert(
            pipeline_id,
            PipelineSupervisor {
                workspace,
                spec,
                tap,
                restart_count: 0,
                current_backoff_ms: initial_backoff,
            },
        );
        drop(g);
        self.mark_dirty();
    }

    /// Variante que preserva backoff/count del supervisor anterior (para
    /// re-registrar tras un restart sin perder el throttle acumulado).
    pub async fn register_pipeline_supervisor_with_state(
        &self,
        pipeline_id: Ulid,
        workspace: WorkspaceId,
        spec: PipelineSpec,
        tap: bool,
        restart_count: u32,
        current_backoff_ms: u64,
    ) {
        if !spec.restart_on_failure {
            return;
        }
        let mut g = self.inner.lock().await;
        g.pipeline_supervisors.insert(
            pipeline_id,
            PipelineSupervisor {
                workspace,
                spec,
                tap,
                restart_count,
                current_backoff_ms,
            },
        );
    }

    /// Drena la cola de pipelines pendientes de restart y retorna las
    /// specs a relaunch. El daemon lo llama tras cada `reap_dead`.
    ///
    /// Aplica `restart_max`: si el supervisor ya pasó el límite, no se
    /// retorna y el supervisor se elimina (give-up). El backoff
    /// preserva el valor actual; el daemon decide cuándo aplicar el
    /// sleep antes del relaunch.
    pub async fn take_pending_restarts(&self) -> Vec<PipelineSupervisor> {
        let mut g = self.inner.lock().await;
        let pending = std::mem::take(&mut g.pending_pipeline_restarts);
        let mut out = Vec::with_capacity(pending.len());
        for old_id in pending {
            if let Some(mut sup) = g.pipeline_supervisors.remove(&old_id) {
                if sup.spec.restart_max > 0 && sup.restart_count >= sup.spec.restart_max {
                    tracing::warn!(
                        label = %sup.spec.label,
                        restart_count = sup.restart_count,
                        max = sup.spec.restart_max,
                        "pipeline restart_max reached — giving up"
                    );
                    continue; // no relaunch, supervisor discarded.
                }
                sup.restart_count += 1;
                out.push(sup);
            }
        }
        out
    }

    /// Registra los comandos lanzados por un pipeline en el workspace.
    /// Esto permite `pipeline_stop` (matar selectivamente sólo los pids
    /// de un pipeline). `pipeline_id` se setea en cada CommandState.
    pub async fn register_pipeline_commands(
        &self,
        workspace: WorkspaceId,
        pipeline_id: Ulid,
        commands: Vec<(String, i32)>,
    ) {
        let mut g = self.inner.lock().await;
        let Some(ws) = g.workspaces.get_mut(&workspace) else { return };
        for (label, pid) in commands {
            let cmd_id = Ulid::new();
            ws.commands.insert(
                cmd_id,
                CommandState {
                    id: cmd_id,
                    label,
                    pid: Pid::from_raw(pid),
                    alive: true,
                    exit_status: None,
                    stdout: None,
                    stderr: None,
                    pipeline_id: Some(pipeline_id),
                },
            );
        }
    }

    /// Detiene selectivamente los comandos de un pipeline. SIGTERM →
    /// `grace` → SIGKILL. Devuelve cantidad reapeada. Si no hay comandos
    /// del pipeline en ningún workspace, retorna 0.
    pub async fn stop_pipeline(
        &self,
        pipeline_id: Ulid,
        grace: std::time::Duration,
    ) -> u32 {
        // 1) Recolectamos pids de ese pipeline en todos los workspaces.
        let mut targets: Vec<Pid> = Vec::new();
        {
            let g = self.inner.lock().await;
            for ws in g.workspaces.values() {
                for cmd in ws.commands.values() {
                    if cmd.alive && cmd.pipeline_id == Some(pipeline_id) {
                        targets.push(cmd.pid);
                    }
                }
            }
        }
        if targets.is_empty() {
            return 0;
        }
        let initial = if grace.is_zero() { Signal::SIGKILL } else { Signal::SIGTERM };
        for pid in &targets {
            let _ = kill(*pid, initial);
        }
        let mut reaped = 0u32;
        let mut still = targets.clone();
        let deadline = std::time::Instant::now() + grace;
        let poll = std::time::Duration::from_millis(20);
        while !still.is_empty() && std::time::Instant::now() < deadline {
            still.retain(|pid| match waitpid(*pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::StillAlive) => true,
                Ok(_) => {
                    reaped += 1;
                    false
                }
                Err(_) => false,
            });
            if !still.is_empty() {
                tokio::time::sleep(poll).await;
            }
        }
        for pid in &still {
            let _ = kill(*pid, Signal::SIGKILL);
            let _ = waitpid(*pid, None);
            reaped += 1;
        }
        // Marcar como dead en estado in-memory.
        let mut g = self.inner.lock().await;
        for ws in g.workspaces.values_mut() {
            for cmd in ws.commands.values_mut() {
                if cmd.pipeline_id == Some(pipeline_id) && cmd.alive {
                    cmd.alive = false;
                }
            }
        }
        // Drop flows del pipeline.
        g.pipeline_flows.remove(&pipeline_id);
        info!(%pipeline_id, reaped, "pipeline stopped");
        reaped
    }

    /// Retiene los FlowChannels de un pipeline para que sobrevivan al
    /// fin del request. Drop = cierre del data plane.
    pub async fn retain_pipeline_flows(
        &self,
        pipeline: Ulid,
        flows: Vec<crate::flow_channel::FlowChannel>,
    ) {
        self.inner.lock().await.pipeline_flows.insert(pipeline, flows);
    }

    /// Snapshot de counts agregados para health endpoint.
    pub async fn health_counts(&self) -> HealthCounts {
        let g = self.inner.lock().await;
        let alive_workspaces = g.workspaces.len() as u32;
        let alive_commands: u32 = g
            .workspaces
            .values()
            .map(|ws| ws.commands.values().filter(|c| c.alive).count() as u32)
            .sum();
        let alive_pipelines = g.pipeline_supervisors.len() as u32;
        let active_flows: u32 = g.pipeline_flows.values().map(|v| v.len() as u32).sum();
        HealthCounts {
            alive_workspaces,
            alive_commands,
            alive_pipelines,
            active_flows,
        }
    }

    /// Lista pipelines vivos con sus sockets activos.
    pub async fn list_flow_pipelines(&self) -> Vec<(Ulid, Vec<std::path::PathBuf>)> {
        let g = self.inner.lock().await;
        g.pipeline_flows
            .iter()
            .map(|(id, flows)| {
                (
                    *id,
                    flows.iter().map(|f| f.socket_path().to_path_buf()).collect(),
                )
            })
            .collect()
    }

    /// Throughput per-socket: bytes_total + bytes_per_sec por flow socket.
    pub async fn flow_throughput(&self) -> Vec<(std::path::PathBuf, u64, f64)> {
        let g = self.inner.lock().await;
        let mut out = Vec::new();
        for flows in g.pipeline_flows.values() {
            for fc in flows {
                out.push((
                    fc.socket_path().to_path_buf(),
                    fc.meter().total_bytes(),
                    fc.meter().bytes_per_sec(),
                ));
            }
        }
        out
    }

    /// Cierra el data plane de un pipeline (drop = remove_file de sockets).
    pub async fn drop_pipeline_flows(&self, pipeline: Ulid) -> bool {
        self.inner.lock().await.pipeline_flows.remove(&pipeline).is_some()
    }

    pub fn incarnator(&self) -> &Incarnator {
        &self.incarnator
    }

    /// Handle Arc-clonable del Incarnator, para que el pipeline lo pueda
    /// usar fuera del manager.
    pub fn incarnator_handle(&self) -> Arc<Incarnator> {
        self.incarnator.clone()
    }

    // -----------------------------------------------------------------
    // Saved pipelines (definiciones nombradas, no runs)
    // -----------------------------------------------------------------

    /// Guarda (o reemplaza) un PipelineSpec bajo `name`.
    pub async fn save_pipeline(&self, name: String, spec: PipelineSpec) {
        self.inner.lock().await.saved_pipelines.insert(name, spec);
        self.mark_dirty();
    }

    /// Devuelve los nombres de los pipelines guardados.
    pub async fn list_saved_pipelines(&self) -> Vec<String> {
        let g = self.inner.lock().await;
        let mut v: Vec<String> = g.saved_pipelines.keys().cloned().collect();
        v.sort();
        v
    }

    /// Recupera el PipelineSpec guardado bajo `name`.
    pub async fn get_saved_pipeline(&self, name: &str) -> Option<PipelineSpec> {
        self.inner.lock().await.saved_pipelines.get(name).cloned()
    }

    /// Elimina un saved pipeline.
    pub async fn drop_saved_pipeline(&self, name: &str) -> bool {
        let existed = self.inner.lock().await.saved_pipelines.remove(name).is_some();
        if existed {
            self.mark_dirty();
        }
        existed
    }

    /// Label del workspace, si existe.
    pub async fn workspace_label(&self, id: WorkspaceId) -> Option<String> {
        self.inner
            .lock()
            .await
            .workspaces
            .get(&id)
            .map(|w| w.spec.label.clone())
    }

    /// Compara accounting real (RSS, commands_alive) contra los rlimits
    /// declarados en `SomaSpec`. Devuelve violaciones humanizadas. NO
    /// hace enforcement automático.
    pub async fn workspace_quota(&self, id: WorkspaceId) -> Option<stats::QuotaReport> {
        let stats_now = self.workspace_stats(id).await?;
        let g = self.inner.lock().await;
        let ws = g.workspaces.get(&id)?;
        let rl = &ws.spec.soma.rlimits;
        let mut report = stats::QuotaReport {
            mem_limit: rl.mem_bytes,
            nproc_limit: rl.nproc,
            breaches: Vec::new(),
        };
        if let (Some(limit), Some(used)) = (rl.mem_bytes, stats_now.rss_bytes) {
            if used > limit {
                report.breaches.push(format!(
                    "memory: {:.2} MiB > {:.2} MiB limit",
                    used as f64 / 1024.0 / 1024.0,
                    limit as f64 / 1024.0 / 1024.0,
                ));
            }
        }
        if let Some(limit) = rl.nproc {
            if stats_now.commands_alive > limit {
                report.breaches.push(format!(
                    "nproc: {} alive > {} limit",
                    stats_now.commands_alive, limit
                ));
            }
        }
        Some(report)
    }

    /// Estadísticas de recursos del workspace: RSS + CPU agregado de sus
    /// comandos vivos. Lee `/proc/<pid>/` directamente; si el spec declara
    /// `soma.cgroup.path`, también intenta el cgroup (más preciso, incluye
    /// descendants).
    ///
    /// `cpu_percent` se calcula entre samples consecutivos. Necesita ≥2
    /// llamadas para tener un valor (la primera siempre retorna `None`).
    pub async fn workspace_stats(&self, id: WorkspaceId) -> Option<stats::WorkspaceStats> {
        let mut g = self.inner.lock().await;
        let ws = g.workspaces.get_mut(&id)?;
        let alive: Vec<i32> = ws
            .commands
            .values()
            .filter(|c| c.alive)
            .map(|c| c.pid.as_raw())
            .collect();
        let total = ws.commands.len() as u32;
        let cgroup_path = if ws.spec.soma.cgroup.path.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(format!(
                "/sys/fs/cgroup{}",
                ws.spec.soma.cgroup.path
            )))
        };
        let mut s = stats::measure(&alive, cgroup_path.as_deref(), ws.started);
        s.commands_total = total;

        // CPU%: diff entre el sample actual y el previo, dividido por
        // wall time. 100% = 1 core saturado. >100% = varios cores.
        let now = Instant::now();
        if let Some(cpu_now) = s.cpu_usec {
            if let Some((prev_t, prev_cpu)) = ws.last_cpu_sample {
                let dt_us = now.duration_since(prev_t).as_micros() as u64;
                let d_cpu = cpu_now.saturating_sub(prev_cpu);
                if dt_us > 0 {
                    s.cpu_percent = Some(100.0 * d_cpu as f32 / dt_us as f32);
                }
            }
            ws.last_cpu_sample = Some((now, cpu_now));
        }
        // Append a history (ring buffer cap).
        if ws.stats_history.len() >= STATS_HISTORY_CAP {
            ws.stats_history.pop_front();
        }
        ws.stats_history.push_back(s.clone());
        Some(s)
    }

    /// Retorna las últimas N samples de stats (servidas desde el ring
    /// buffer interno). Sobrevive restart del shell.
    pub async fn workspace_stats_history(
        &self,
        id: WorkspaceId,
        tail: usize,
    ) -> Option<Vec<stats::WorkspaceStats>> {
        let g = self.inner.lock().await;
        let ws = g.workspaces.get(&id)?;
        let take = if tail == 0 { ws.stats_history.len() } else { tail };
        let skip = ws.stats_history.len().saturating_sub(take);
        Some(ws.stats_history.iter().skip(skip).cloned().collect())
    }

    pub async fn create(
        self: &Arc<Self>,
        spec: WorkspaceSpec,
    ) -> Result<(WorkspaceId, Vec<String>), CoreError> {
        self.create_with_id(WorkspaceId::new(), spec).await
    }

    /// Variante que acepta el ID. Útil para restore_snapshot: preserva
    /// ULIDs entre restarts, así clients que tracking workspace_id no se
    /// rompen.
    pub async fn create_with_id(
        self: &Arc<Self>,
        id: WorkspaceId,
        spec: WorkspaceSpec,
    ) -> Result<(WorkspaceId, Vec<String>), CoreError> {
        let card = spec.to_card(id)?;
        let mut warnings = self.incarnator.dry_run(&card).warnings;
        let ttl = spec.ttl;

        // Si el workspace declara cgroup path Y rlimits, intentamos
        // crear el cgroup y escribir memory.max/pids.max. El kernel
        // hace OOM kill al exceder memory.max — enforcement automático
        // sin policy adicional. Falla silenciosa si no hay delegation.
        if !spec.soma.cgroup.path.is_empty() {
            if let Ok(abs) = ente_incarnate::cgroup::ensure_cgroup(&spec.soma.cgroup) {
                let applied =
                    ente_incarnate::cgroup::apply_rlimits_to_cgroup(&abs, &spec.soma.rlimits);
                if !applied.is_empty() {
                    warnings.push(format!("cgroup limits applied: {}", applied.join(", ")));
                }
            }
        }
        let state = WorkspaceState {
            id,
            spec,
            root_card: card,
            commands: HashMap::new(),
            started: Instant::now(),
            last_cpu_sample: None,
            stats_history: std::collections::VecDeque::with_capacity(STATS_HISTORY_CAP),
        };
        self.inner.lock().await.workspaces.insert(id, state);
        self.mark_dirty();
        info!(%id, ?ttl, "workspace created");

        // Si tiene TTL, programar auto-stop. El task captura un weak ref
        // al manager para no impedir que se dropée si el daemon termina.
        if let Some(duration) = ttl {
            let mgr_weak = Arc::downgrade(self);
            tokio::spawn(async move {
                tokio::time::sleep(duration).await;
                if let Some(mgr) = mgr_weak.upgrade() {
                    let exists = mgr.inner.lock().await.workspaces.contains_key(&id);
                    if exists {
                        info!(%id, "workspace TTL expired — auto-stop");
                        let _ = mgr.stop(id).await;
                    }
                }
            });
        }

        Ok((id, warnings))
    }

    pub async fn list(&self) -> Vec<WorkspaceSnapshot> {
        let g = self.inner.lock().await;
        g.workspaces
            .values()
            .map(|w| WorkspaceSnapshot {
                id: w.id,
                label: w.spec.label.clone(),
                commands: w.commands.len() as u32,
                uptime_ms: w.started.elapsed().as_millis() as u64,
            })
            .collect()
    }

    pub async fn stop(&self, id: WorkspaceId) -> Result<u32, CoreError> {
        self.stop_with_grace(id, std::time::Duration::from_millis(1000)).await
    }

    /// Variante con tiempo de gracia configurable. SIGTERM → espera `grace`
    /// → SIGKILL si quedan vivos. `grace=0` = SIGKILL inmediato.
    pub async fn stop_with_grace(
        &self,
        id: WorkspaceId,
        grace: std::time::Duration,
    ) -> Result<u32, CoreError> {
        let mut g = self.inner.lock().await;
        let ws = g.workspaces.remove(&id).ok_or(CoreError::WorkspaceNotFound(id))?;
        // También limpiamos flow_channels del workspace si los hubiera —
        // por workspace lo retenemos por pipeline, no por workspace.
        drop(g);
        self.mark_dirty();

        // 1) SIGTERM (o SIGKILL si grace=0) a todos vivos.
        let initial_signal = if grace.is_zero() { Signal::SIGKILL } else { Signal::SIGTERM };
        let alive_pids: Vec<Pid> = ws
            .commands
            .values()
            .filter(|c| c.alive)
            .map(|c| c.pid)
            .collect();
        for pid in &alive_pids {
            let _ = kill(*pid, initial_signal);
        }

        // 2) Esperar hasta `grace` haciendo polling WNOHANG.
        let mut reaped = 0u32;
        let mut still_alive: Vec<Pid> = alive_pids.clone();
        let deadline = std::time::Instant::now() + grace;
        let poll_interval = std::time::Duration::from_millis(20);
        while !still_alive.is_empty() && std::time::Instant::now() < deadline {
            still_alive.retain(|pid| match waitpid(*pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::StillAlive) => true,
                Ok(_) => {
                    reaped += 1;
                    false
                }
                Err(_) => false,
            });
            if !still_alive.is_empty() {
                tokio::time::sleep(poll_interval).await;
            }
        }

        // 3) SIGKILL forzoso a los que quedan, y wait blocking.
        for pid in &still_alive {
            let _ = kill(*pid, Signal::SIGKILL);
            let _ = waitpid(*pid, None);
            reaped += 1;
        }
        info!(
            %id,
            reaped,
            grace_ms = grace.as_millis() as u64,
            sigkilled = still_alive.len(),
            "workspace stopped"
        );
        Ok(reaped)
    }

    /// Ejecuta un comando one-shot dentro de un workspace existente.
    /// Captura stdout+stderr en un ring buffer accesible vía
    /// [`get_command_logs`](Self::get_command_logs).
    pub async fn run(
        &self,
        id: WorkspaceId,
        exec: String,
        argv: Vec<String>,
        envp: Vec<(String, String)>,
    ) -> Result<CommandSummary, CoreError> {
        self.run_with_options(id, exec, argv, envp, false).await
    }

    /// Variante con `restart_on_failure`: si el comando muere con
    /// exit_status != 0, el reaper lo relauncha con backoff exponencial
    /// (200ms → 400 → 800 → … cap 30s).
    pub async fn run_with_options(
        &self,
        id: WorkspaceId,
        exec: String,
        argv: Vec<String>,
        envp: Vec<(String, String)>,
        restart_on_failure: bool,
    ) -> Result<CommandSummary, CoreError> {
        let workspace_label = {
            let g = self.inner.lock().await;
            let ws = g.workspaces.get(&id).ok_or(CoreError::WorkspaceNotFound(id))?;
            ws.spec.label.clone()
        };
        let cmd_ref = CommandRef {
            label: format!("run-{}", short_ulid(&Ulid::new())),
            payload: Payload::Native { exec, argv, envp },
            soma: Default::default(),
            flows: Default::default(),
            supervision: Supervision::OneShot,
        };
        let card = cmd_ref.to_card(0, &workspace_label)?;

        // Dos pipes O_CLOEXEC: uno para stdout, otro para stderr.
        use std::os::fd::IntoRawFd;
        let (sout_r, sout_w) =
            nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC).map_err(|e| {
                CoreError::Incarnate(ente_incarnate::IncarnateError::Pipe(e))
            })?;
        let (serr_r, serr_w) =
            nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC).map_err(|e| {
                CoreError::Incarnate(ente_incarnate::IncarnateError::Pipe(e))
            })?;
        let sout_r_fd = sout_r.into_raw_fd();
        let sout_w_fd = sout_w.into_raw_fd();
        let serr_r_fd = serr_r.into_raw_fd();
        let serr_w_fd = serr_w.into_raw_fd();

        let stdout_buf = logbuf::LogBuf::new();
        let stderr_buf = logbuf::LogBuf::new();

        let stdio = ente_incarnate::ChildStdio {
            stdin_fd: None,
            stdout_fd: Some(sout_w_fd),
            stderr_fd: Some(serr_w_fd),
        };
        let out = self.incarnator.incarnate_with(&card, stdio)?;
        let cmd_id = card.id;
        let cmd_label = cmd_ref.label.clone();
        let pid = out.pid;

        spawn_log_drainer(sout_r_fd, stdout_buf.clone());
        spawn_log_drainer(serr_r_fd, stderr_buf.clone());

        let mut g = self.inner.lock().await;
        if let Some(ws) = g.workspaces.get_mut(&id) {
            ws.commands.insert(
                cmd_id,
                CommandState {
                    id: cmd_id,
                    label: cmd_label.clone(),
                    pid,
                    alive: true,
                    exit_status: None,
                    stdout: Some(stdout_buf),
                    stderr: Some(stderr_buf),
                    pipeline_id: None,
                },
            );
        }
        if restart_on_failure {
            // Reextract exec/argv/envp del payload del CommandRef.
            if let Payload::Native { exec, argv, envp } = &cmd_ref.payload {
                g.restart_specs.insert(
                    cmd_id,
                    RestartSpec {
                        workspace: id,
                        exec: exec.clone(),
                        argv: argv.clone(),
                        envp: envp.clone(),
                        backoff_ms: 200,
                        max_backoff_ms: 30_000,
                        restart_count: 0,
                    },
                );
            }
        }
        for d in &out.degradations {
            warn!(?d, %id, "command incarnation degradation");
        }
        Ok(CommandSummary {
            id: cmd_id,
            label: cmd_label,
            pid: pid.as_raw(),
        })
    }

    /// Devuelve el tail del log capturado para `(workspace, command)`.
    /// `stream` selecciona stdout/stderr/both.
    pub async fn get_command_logs(
        &self,
        workspace: WorkspaceId,
        command: Ulid,
        tail_bytes: usize,
        stream: LogStream,
    ) -> Option<Vec<u8>> {
        let g = self.inner.lock().await;
        let ws = g.workspaces.get(&workspace)?;
        let cmd = ws.commands.get(&command)?;
        match stream {
            LogStream::Stdout => cmd.stdout.as_ref().map(|lb| lb.tail(tail_bytes)),
            LogStream::Stderr => cmd.stderr.as_ref().map(|lb| lb.tail(tail_bytes)),
            LogStream::Both => {
                let so = cmd.stdout.as_ref().map(|lb| lb.tail(tail_bytes)).unwrap_or_default();
                let se = cmd.stderr.as_ref().map(|lb| lb.tail(tail_bytes)).unwrap_or_default();
                let mut out = so;
                out.extend_from_slice(&se);
                Some(out)
            }
        }
    }

    /// Lista comandos de un workspace.
    pub async fn list_commands(&self, workspace: WorkspaceId) -> Vec<CommandInfo> {
        let g = self.inner.lock().await;
        let Some(ws) = g.workspaces.get(&workspace) else { return Vec::new() };
        let mut out: Vec<CommandInfo> = ws
            .commands
            .values()
            .map(|c| CommandInfo {
                id: c.id,
                label: c.label.clone(),
                pid: c.pid.as_raw(),
                alive: c.alive,
                exit_status: c.exit_status,
                log_bytes: c.stdout.as_ref().map(|l| l.written_total()).unwrap_or(0)
                    + c.stderr.as_ref().map(|l| l.written_total()).unwrap_or(0),
            })
            .collect();
        // Orden estable por ULID (temporal).
        out.sort_by_key(|c| c.id);
        out
    }

    /// Lanza todas las Cards de un Pipeline. Devuelve (label, pid) por nodo.
    /// La conexión via flows queda librada al broker (cuando haya integración
    /// completa con sidecar; v1 sólo lanza).
    pub async fn run_pipeline(
        &self,
        spec: &PipelineSpec,
    ) -> Result<Vec<(String, Pid)>, CoreError> {
        spec.validate()?;
        let workspace_label = {
            let g = self.inner.lock().await;
            let ws = g
                .workspaces
                .get(&spec.workspace)
                .ok_or(CoreError::WorkspaceNotFound(spec.workspace))?;
            ws.spec.label.clone()
        };
        let mut launched = Vec::new();
        for (i, node) in spec.nodes.iter().enumerate() {
            let card = node.to_card(i, &workspace_label)?;
            let out = self.incarnator.incarnate(&card)?;
            let mut g = self.inner.lock().await;
            if let Some(ws) = g.workspaces.get_mut(&spec.workspace) {
                ws.commands.insert(
                    card.id,
                    CommandState {
                        id: card.id,
                        label: node.label.clone(),
                        pid: out.pid,
                        alive: true,
                        exit_status: None,
                        stdout: None, // run_pipeline NO captura (conecta por pipes).
                        stderr: None,
                        pipeline_id: None,
                    },
                );
            }
            launched.push((node.label.clone(), out.pid));
        }
        Ok(launched)
    }

    /// Cosecha hijos terminados (no-bloqueante). Llamar periódicamente desde
    /// el daemon o ante SIGCHLD. Marca `alive=false` y guarda exit_status.
    pub async fn reap_dead(self: &Arc<Self>) {
        let mut to_restart: Vec<RestartSpec> = Vec::new();
        let mut to_enforce_kill: Vec<WorkspaceId> = Vec::new();
        {
            let mut g = self.inner.lock().await;
            for ws in g.workspaces.values_mut() {
                for cmd in ws.commands.values_mut() {
                    if !cmd.alive {
                        continue;
                    }
                    match waitpid(cmd.pid, Some(WaitPidFlag::WNOHANG)) {
                        Ok(WaitStatus::Exited(_, code)) => {
                            cmd.alive = false;
                            cmd.exit_status = Some(code);
                        }
                        Ok(WaitStatus::Signaled(_, sig, _)) => {
                            cmd.alive = false;
                            cmd.exit_status = Some(128 + (sig as i32));
                        }
                        _ => {}
                    }
                }
            }
            // Quota enforcement: chequear breach por workspace y aplicar policy.
            // Lo hacemos dentro del mismo lock para tener una lectura
            // consistente; el kill real va fuera del lock.
            for (ws_id, ws) in g.workspaces.iter() {
                let rl = &ws.spec.soma.rlimits;
                let qe = &ws.spec.quota_enforce;
                // Sólo aplicamos si hay al menos una action != None.
                if qe.mem == shuma_card::QuotaAction::None
                    && qe.nproc == shuma_card::QuotaAction::None
                {
                    continue;
                }
                // Medir RSS y nproc vivos sin pasar por workspace_stats
                // (que tomaría el lock recursivo). Hacemos un read directo.
                let alive: Vec<i32> = ws
                    .commands
                    .values()
                    .filter(|c| c.alive)
                    .map(|c| c.pid.as_raw())
                    .collect();
                let nproc_alive = alive.len() as u32;
                let mem_used: u64 = alive
                    .iter()
                    .filter_map(|pid| read_proc_rss(*pid))
                    .sum();

                let mem_breach = matches!(rl.mem_bytes, Some(limit) if mem_used > limit);
                let nproc_breach = matches!(rl.nproc, Some(limit) if nproc_alive > limit);

                let mut kill_needed = false;
                if mem_breach {
                    match qe.mem {
                        shuma_card::QuotaAction::Log => {
                            warn!(%ws_id, used = mem_used, limit = ?rl.mem_bytes, "quota breach: memory");
                        }
                        shuma_card::QuotaAction::Kill => {
                            warn!(%ws_id, used = mem_used, limit = ?rl.mem_bytes, "quota breach: KILLING");
                            kill_needed = true;
                        }
                        _ => {}
                    }
                }
                if nproc_breach {
                    match qe.nproc {
                        shuma_card::QuotaAction::Log => {
                            warn!(%ws_id, alive = nproc_alive, limit = ?rl.nproc, "quota breach: nproc");
                        }
                        shuma_card::QuotaAction::Kill => {
                            warn!(%ws_id, alive = nproc_alive, limit = ?rl.nproc, "quota breach: KILLING");
                            kill_needed = true;
                        }
                        _ => {}
                    }
                }
                if kill_needed {
                    to_enforce_kill.push(*ws_id);
                }
            }
            // Pipeline supervisor: detectar pipelines cuyos comandos tienen
            // failure. Marca para restart si tiene supervisor.
            // Esto se hace cuando TODOS los comandos del pipeline están
            // dead Y al menos uno tiene exit!=0 (sino podría disparar
            // restart mientras otros comandos aún corren — incorrecto).
            let supervisor_ids: Vec<Ulid> = g.pipeline_supervisors.keys().copied().collect();
            for pipe_id in supervisor_ids {
                // ¿Hay algún comando vivo de este pipeline?
                let mut all_dead = true;
                let mut any_failed = false;
                for ws in g.workspaces.values() {
                    for cmd in ws.commands.values() {
                        if cmd.pipeline_id != Some(pipe_id) {
                            continue;
                        }
                        if cmd.alive {
                            all_dead = false;
                        } else if cmd.exit_status.map_or(false, |s| s != 0) {
                            any_failed = true;
                        }
                    }
                }
                if all_dead && any_failed {
                    // Push a queue si no estaba ya.
                    if !g.pending_pipeline_restarts.contains(&pipe_id) {
                        g.pending_pipeline_restarts.push(pipe_id);
                    }
                }
            }
            // Detectar restart_specs cuyo command_id ya está dead con exit!=0.
            let mut to_remove: Vec<Ulid> = Vec::new();
            for (cmd_id, spec) in g.restart_specs.iter() {
                let mut should_restart = false;
                let mut should_drop = false;
                'outer: for ws in g.workspaces.values() {
                    if let Some(cmd) = ws.commands.get(cmd_id) {
                        if !cmd.alive {
                            match cmd.exit_status {
                                Some(0) => should_drop = true,
                                Some(_) => should_restart = true,
                                None => {}
                            }
                            break 'outer;
                        }
                    }
                }
                if should_drop {
                    to_remove.push(*cmd_id);
                } else if should_restart {
                    to_restart.push(spec.clone());
                    to_remove.push(*cmd_id);
                }
            }
            for id in to_remove {
                g.restart_specs.remove(&id);
            }
        }
        // Quota enforcement: kill workspaces fuera del lock.
        for ws_id in to_enforce_kill {
            let _ = self.stop_with_grace(ws_id, std::time::Duration::ZERO).await;
        }
        // Schedule restart fuera del lock.
        for mut spec in to_restart {
            let mgr = self.clone();
            let backoff = std::time::Duration::from_millis(spec.backoff_ms);
            // Subir el backoff para la PRÓXIMA falla, no esta.
            spec.backoff_ms = (spec.backoff_ms * 2).min(spec.max_backoff_ms);
            spec.restart_count += 1;
            let restart_n = spec.restart_count;
            tokio::spawn(async move {
                tokio::time::sleep(backoff).await;
                info!(
                    backoff_ms = backoff.as_millis() as u64,
                    restart = restart_n,
                    "restarting failed command"
                );
                let workspace = spec.workspace;
                if let Err(e) = mgr
                    .run_with_options(workspace, spec.exec.clone(), spec.argv.clone(), spec.envp.clone(), true)
                    .await
                {
                    warn!(?e, "restart failed to launch");
                    return;
                }
                // Preservar backoff acumulado: localizar el nuevo command_id
                // (el más reciente vivo en el workspace) y sobreescribir.
                let new_cmd_id = {
                    let g = mgr.inner.lock().await;
                    g.workspaces.get(&workspace).and_then(|ws| {
                        ws.commands
                            .values()
                            .filter(|c| c.alive)
                            .max_by_key(|c| c.id)
                            .map(|c| c.id)
                    })
                };
                if let Some(new_id) = new_cmd_id {
                    let mut g = mgr.inner.lock().await;
                    if let Some(existing) = g.restart_specs.get_mut(&new_id) {
                        existing.backoff_ms = spec.backoff_ms;
                        existing.restart_count = spec.restart_count;
                    }
                }
            });
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ttl_auto_stops_workspace() {
        let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
        let spec = WorkspaceSpec {
            label: "ttl-test".into(),
            soma: Default::default(),
            permissions: Default::default(),
            ttl: Some(std::time::Duration::from_millis(120)),
            flow_dirs: vec![],
            on_exit: shuma_card::ExitPolicy::Reap,
            quota_enforce: Default::default(),
        };
        let (id, _) = mgr.create(spec).await.unwrap();
        assert_eq!(mgr.list().await.len(), 1);
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        assert_eq!(
            mgr.list().await.len(),
            0,
            "TTL expirado: workspace debe haber sido removido"
        );
        let _ = id;
    }

    #[tokio::test]
    async fn create_and_list_workspace() {
        let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
        let spec = WorkspaceSpec {
            label: "test".into(),
            soma: Default::default(),
            permissions: Default::default(),
            ttl: None,
            flow_dirs: vec![],
            on_exit: shuma_card::ExitPolicy::Reap,
            quota_enforce: Default::default(),
        };
        let (id, _w) = mgr.create(spec).await.unwrap();
        let list = mgr.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
    }

    #[tokio::test]
    async fn run_captures_stdout_to_log() {
        let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
        let spec = WorkspaceSpec {
            label: "logs".into(),
            soma: Default::default(),
            permissions: Default::default(),
            ttl: None,
            flow_dirs: vec![],
            on_exit: shuma_card::ExitPolicy::Reap,
            quota_enforce: Default::default(),
        };
        let (id, _) = mgr.create(spec).await.unwrap();
        let summary = mgr
            .run(id, "/bin/echo".into(), vec!["captured-output".into()], vec![])
            .await
            .unwrap();
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            mgr.reap_dead().await;
            let logs = mgr
                .get_command_logs(id, summary.id, 0, LogStream::Stdout)
                .await
                .unwrap_or_default();
            if !logs.is_empty() {
                let s = String::from_utf8_lossy(&logs);
                assert!(s.contains("captured-output"), "got: {s:?}");
                return;
            }
        }
        panic!("logs never captured");
    }

    #[tokio::test]
    async fn run_captures_stderr_separately() {
        let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
        let spec = WorkspaceSpec {
            label: "stderr".into(),
            soma: Default::default(),
            permissions: Default::default(),
            ttl: None,
            flow_dirs: vec![],
            on_exit: shuma_card::ExitPolicy::Reap,
            quota_enforce: Default::default(),
        };
        let (id, _) = mgr.create(spec).await.unwrap();
        // sh -c "echo OUT; echo ERR >&2"
        let summary = mgr
            .run(
                id,
                "/bin/sh".into(),
                vec!["-c".into(), "echo OUT; echo ERR >&2".into()],
                vec![],
            )
            .await
            .unwrap();
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            mgr.reap_dead().await;
            let so = mgr
                .get_command_logs(id, summary.id, 0, LogStream::Stdout)
                .await
                .unwrap_or_default();
            let se = mgr
                .get_command_logs(id, summary.id, 0, LogStream::Stderr)
                .await
                .unwrap_or_default();
            if !so.is_empty() && !se.is_empty() {
                let so_s = String::from_utf8_lossy(&so);
                let se_s = String::from_utf8_lossy(&se);
                assert!(so_s.contains("OUT"), "stdout: {so_s:?}");
                assert!(se_s.contains("ERR"), "stderr: {se_s:?}");
                assert!(!so_s.contains("ERR"), "stdout no debería tener ERR");
                assert!(!se_s.contains("OUT"), "stderr no debería tener OUT");
                return;
            }
        }
        panic!("logs never captured on both streams");
    }

    #[tokio::test]
    async fn restart_on_failure_relaunches_failing_command() {
        let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
        let spec = WorkspaceSpec {
            label: "restart".into(),
            soma: Default::default(),
            permissions: Default::default(),
            ttl: None,
            flow_dirs: vec![],
            on_exit: shuma_card::ExitPolicy::Reap,
            quota_enforce: Default::default(),
        };
        let (id, _) = mgr.create(spec).await.unwrap();
        // /bin/false sale con exit=1. Con restart_on_failure=true debería
        // relanzarse al menos 1 vez (tras el backoff inicial de 200ms).
        let summary = mgr
            .run_with_options(id, "/bin/false".into(), vec![], vec![], true)
            .await
            .unwrap();
        let original_id = summary.id;
        // Esperamos ~500ms para que termine + reap + restart corra.
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            mgr.reap_dead().await;
            let g = mgr.inner.lock().await;
            if let Some(ws) = g.workspaces.get(&id) {
                let new_cmds: Vec<_> = ws.commands.keys().filter(|k| **k != original_id).collect();
                if !new_cmds.is_empty() {
                    // Hay un nuevo command_id → restart funcionó.
                    return;
                }
            }
        }
        panic!("restart never launched a new command");
    }

    #[tokio::test]
    async fn pipeline_supervisor_queues_restart_on_failure() {
        use shuma_card::{CommandRef, DiscernPolicy, PipelineSpec};
        let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
        let (ws_id, _) = mgr.create(WorkspaceSpec {
            label: "psup".into(),
            soma: Default::default(),
            permissions: Default::default(),
            ttl: None,
            flow_dirs: vec![],
            on_exit: shuma_card::ExitPolicy::Reap,
            quota_enforce: Default::default(),
        }).await.unwrap();
        let spec = PipelineSpec {
            label: "fail-pipeline".into(),
            workspace: ws_id,
            nodes: vec![CommandRef {
                label: "boom".into(),
                payload: brahman_card::Payload::Native {
                    exec: "/bin/false".into(),
                    argv: vec![],
                    envp: vec![],
                },
                soma: Default::default(),
                flows: Default::default(),
                supervision: brahman_card::Supervision::OneShot,
            }],
            edges: vec![],
            discern: DiscernPolicy::default(),
            restart_on_failure: true,
            restart_backoff_ms: 200,
            restart_max_backoff_ms: 30_000,
            restart_max: 0,
        };
        let pipeline_id = ulid::Ulid::new();
        // Simulamos lo que haría el daemon: registramos un comando como
        // si fuera de pipeline. Usamos `register_pipeline_commands` con
        // un pid fake — pero como reaper hace waitpid, mejor lanzar de verdad.
        // Hack: usar /bin/false via run() y manualmente marcar pipeline_id.
        let summary = mgr.run(ws_id, "/bin/false".into(), vec![], vec![]).await.unwrap();
        // Marcar el comando con pipeline_id manualmente.
        {
            let mut g = mgr.inner.lock().await;
            if let Some(ws) = g.workspaces.get_mut(&ws_id) {
                if let Some(cmd) = ws.commands.get_mut(&summary.id) {
                    cmd.pipeline_id = Some(pipeline_id);
                }
            }
        }
        mgr.register_pipeline_supervisor(pipeline_id, ws_id, spec, true).await;
        // Esperamos que reap detecte la falla y push a pending.
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            mgr.reap_dead().await;
            let pending = mgr.take_pending_restarts().await;
            if !pending.is_empty() {
                assert_eq!(pending[0].spec.label, "fail-pipeline");
                return;
            }
        }
        panic!("supervisor never queued a restart");
    }

    #[tokio::test]
    async fn quota_enforce_nproc_kill_terminates_commands() {
        let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
        let mut spec = WorkspaceSpec {
            label: "qenforce".into(),
            soma: Default::default(),
            permissions: Default::default(),
            ttl: None,
            flow_dirs: vec![],
            on_exit: shuma_card::ExitPolicy::Reap,
            quota_enforce: shuma_card::QuotaEnforcement {
                mem: shuma_card::QuotaAction::None,
                nproc: shuma_card::QuotaAction::Kill,
            },
        };
        spec.soma.rlimits.nproc = Some(1);
        let (id, _) = mgr.create(spec).await.unwrap();
        // Lanzo 2 procesos (cada uno sleep). nproc_limit=1 → breach inmediato.
        let _ = mgr.run(id, "/bin/sleep".into(), vec!["5".into()], vec![]).await.unwrap();
        let _ = mgr.run(id, "/bin/sleep".into(), vec!["5".into()], vec![]).await.unwrap();
        // Reaper detecta breach y mata workspace.
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            mgr.reap_dead().await;
            let alive = mgr.list().await;
            if alive.is_empty() {
                return; // workspace removido por stop()
            }
        }
        panic!("quota enforce kill never triggered");
    }

    #[tokio::test]
    async fn workspace_stats_history_accumulates() {
        let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
        let spec = WorkspaceSpec {
            label: "history".into(),
            soma: Default::default(),
            permissions: Default::default(),
            ttl: None,
            flow_dirs: vec![],
            on_exit: shuma_card::ExitPolicy::Reap,
            quota_enforce: Default::default(),
        };
        let (id, _) = mgr.create(spec).await.unwrap();
        // Necesitamos al menos un comando vivo para que `measure` no
        // retorne source=none (que igual se appendea, pero con stats vacíos).
        let _ = mgr
            .run(id, "/bin/sleep".into(), vec!["5".into()], vec![])
            .await
            .unwrap();
        // Llamar stats 5 veces.
        for _ in 0..5 {
            let _ = mgr.workspace_stats(id).await;
        }
        let history = mgr.workspace_stats_history(id, 0).await.unwrap();
        assert_eq!(history.len(), 5, "history debería tener 5 samples");
        // tail=3 retorna los últimos 3.
        let tail3 = mgr.workspace_stats_history(id, 3).await.unwrap();
        assert_eq!(tail3.len(), 3);
        // Cleanup.
        let _ = mgr.stop_with_grace(id, std::time::Duration::ZERO).await;
    }

    #[tokio::test]
    async fn run_true_in_workspace() {
        let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
        let spec = WorkspaceSpec {
            label: "exec".into(),
            soma: Default::default(),
            permissions: Default::default(),
            ttl: None,
            flow_dirs: vec![],
            on_exit: shuma_card::ExitPolicy::Reap,
            quota_enforce: Default::default(),
        };
        let (id, _) = mgr.create(spec).await.unwrap();
        let summary = mgr
            .run(id, "/bin/true".into(), vec![], vec![])
            .await
            .unwrap();
        assert!(summary.pid > 0);
        // Cosecha.
        std::thread::sleep(std::time::Duration::from_millis(100));
        mgr.reap_dead().await;
    }
}
