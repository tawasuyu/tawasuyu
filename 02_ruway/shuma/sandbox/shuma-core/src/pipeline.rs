//! Pipeline runtime: encadena nodos con pipes y opcionalmente intercepta
//! cada flow para discernir su contenido.
//!
//! Cada nodo se encarna via [`arje_incarnate::Incarnator`] — eso significa
//! que **cada comando puede tener su propio SomaSpec** (namespaces, cgroup,
//! rlimits) heredado del workspace. La conexión stdin↔stdout se hace con
//! `pipe2(2)` + `ChildStdio` declarativo: el callback de clone(2) hace los
//! `dup2` pre-execve sin romper la regla async-signal-safe.

use crate::CoreError;
use brahman_card::Payload;
use arje_incarnate::{ChildStdio, Incarnator};
use nix::fcntl::OFlag;
use nix::unistd::pipe2;
use shuma_card::PipelineSpec;
use shuma_discern::{DiscernPipeline, Discernment, Hint};
use std::os::fd::{AsRawFd, IntoRawFd, RawFd};
use std::sync::Arc;
use tokio::io::unix::AsyncFd;
use tokio::io::Interest;
use tracing::{debug, info, warn};
use ulid::Ulid;

/// Resultado de lanzar un pipeline.
#[derive(Debug)]
pub struct PipelineLaunch {
    pub pipeline: Ulid,
    pub command_pids: Vec<(String, i32)>,
    /// Discernments por edge, en el mismo orden que `spec.edges`.
    pub edge_discernments: Vec<EdgeDiscernment>,
}

#[derive(Debug, Clone)]
pub struct EdgeDiscernment {
    pub from_label: String,
    pub from_output: String,
    pub to_label: String,
    pub to_input: String,
    pub discernment: Option<Discernment>,
    /// Path del Unix socket donde otros módulos pueden suscribirse al
    /// stream replicado por este edge. `None` cuando tap=false (no hay
    /// data plane porque no hay sampling).
    pub flow_socket: Option<std::path::PathBuf>,
}

/// Lanza un pipeline conectando nodos por stdin/stdout. Cada nodo se
/// encarna via `Incarnator` (con o sin namespacing según su SomaSpec).
///
/// Soporta:
/// - Pipeline lineal (1 producer → 1 consumer).
/// - **Fan-out** (1 producer → N consumers): shuma interpone un
///   splitter que duplica bytes a cada destino. Cuando `tap=true`, el
///   splitter además samplea para discernir.
/// - Múltiples predecessors por nodo NO se soporta aún (fan-in): sólo se
///   honra el primer edge entrante.
pub async fn run_pipeline(
    spec: &PipelineSpec,
    workspace_label: &str,
    tap: bool,
    discerner: Arc<DiscernPipeline>,
    incarnator: Arc<Incarnator>,
    manager: Option<Arc<crate::WorkspaceManager>>,
) -> Result<PipelineLaunch, CoreError> {
    spec.validate()?;
    let n = spec.nodes.len();
    info!(
        nodes = n,
        edges = spec.edges.len(),
        tap,
        "launching pipeline (incarnated)"
    );

    // Pre-compute grafo:
    // - `consumers[i]` = índices de edges salientes de `i`.
    // - `predecessors[j]` = índices de edges entrantes a `j`.
    let mut consumers: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (idx, e) in spec.edges.iter().enumerate() {
        consumers[e.from].push(idx);
        predecessors[e.to].push(idx);
    }

    // Por cada edge: par (r_to_consumer, w_from_producer_side).
    // El consumer recibe r_to_consumer; el producer escribe a w_from_producer_side
    // (directa o vía splitter).
    let mut edge_r: Vec<RawFd> = vec![-1; spec.edges.len()];
    let mut edge_w: Vec<RawFd> = vec![-1; spec.edges.len()];
    for i in 0..spec.edges.len() {
        let (r, w) = pipe2(OFlag::O_CLOEXEC).map_err(|e| {
            CoreError::Incarnate(arje_incarnate::IncarnateError::Pipe(e))
        })?;
        edge_r[i] = r.into_raw_fd();
        edge_w[i] = w.into_raw_fd();
    }

    let mut consumer_stdin_fd: Vec<Option<RawFd>> = vec![None; n];
    let mut producer_stdout_fd: Vec<Option<RawFd>> = vec![None; n];
    let mut splitter_specs: Vec<SplitterSpec> = Vec::new();
    let mut merger_specs: Vec<MergerSpec> = Vec::new();

    // Stdout del producer: directo a edge_w[único] si tiene 1 consumer y NO tap;
    // sino, pipe propio que va al splitter task.
    for i in 0..n {
        if consumers[i].is_empty() {
            continue;
        }
        if consumers[i].len() == 1 && !tap {
            producer_stdout_fd[i] = Some(edge_w[consumers[i][0]]);
            continue;
        }
        // Splitter: pipe propio para el productor → splitter lee y replica a edge_w[*].
        let (prod_r, prod_w) = pipe2(OFlag::O_CLOEXEC).map_err(|e| {
            CoreError::Incarnate(arje_incarnate::IncarnateError::Pipe(e))
        })?;
        producer_stdout_fd[i] = Some(prod_w.into_raw_fd());
        let prod_r_fd = prod_r.into_raw_fd();
        let mut consumer_writes: Vec<RawFd> = Vec::with_capacity(consumers[i].len());
        let mut edge_meta: Vec<EdgeMeta> = Vec::with_capacity(consumers[i].len());
        for edge_idx in &consumers[i] {
            let edge = &spec.edges[*edge_idx];
            consumer_writes.push(edge_w[*edge_idx]);
            edge_meta.push(EdgeMeta {
                from_label: spec.nodes[edge.from].label.clone(),
                from_output: edge.from_output.clone(),
                to_label: spec.nodes[edge.to].label.clone(),
                to_input: edge.to_input.clone(),
            });
        }
        splitter_specs.push(SplitterSpec {
            producer_r_fd: prod_r_fd,
            consumer_w_fds: consumer_writes,
            edges: edge_meta,
            tap,
            sample_bytes: spec.discern.sample_bytes,
            max_bytes_per_sec: spec.discern.max_bytes_per_sec,
        });
    }

    // Stdin del consumer: edge_r[único] si tiene 1 predecessor; sino, merger.
    for j in 0..n {
        match predecessors[j].len() {
            0 => {}
            1 => {
                consumer_stdin_fd[j] = Some(edge_r[predecessors[j][0]]);
            }
            _ => {
                // Merger: lee de N edge_r y escribe a un nuevo pipe cuyo
                // read end es el stdin del consumer.
                let (cons_r, cons_w) = pipe2(OFlag::O_CLOEXEC).map_err(|e| {
                    CoreError::Incarnate(arje_incarnate::IncarnateError::Pipe(e))
                })?;
                consumer_stdin_fd[j] = Some(cons_r.into_raw_fd());
                let inputs: Vec<RawFd> = predecessors[j]
                    .iter()
                    .map(|eidx| edge_r[*eidx])
                    .collect();
                merger_specs.push(MergerSpec {
                    producer_r_fds: inputs,
                    consumer_w_fd: cons_w.into_raw_fd(),
                });
            }
        }
    }

    // Encarnamos cada nodo con su stdin/stdout fd asignado.
    let mut pids = Vec::with_capacity(n);
    for (i, node) in spec.nodes.iter().enumerate() {
        match &node.payload {
            Payload::Native { .. } | Payload::Legacy { .. } => {}
            _ => {
                return Err(CoreError::Incarnate(
                    arje_incarnate::IncarnateError::NonExecutablePayload,
                ))
            }
        }
        let card = node.to_card(i, workspace_label)?;
        let stdio = ChildStdio {
            stdin_fd: consumer_stdin_fd[i],
            stdout_fd: producer_stdout_fd[i],
            stderr_fd: None,
        };
        let outcome = incarnator
            .incarnate_with(&card, stdio)
            .map_err(CoreError::Incarnate)?;
        let pid = outcome.pid;
        pids.push((node.label.clone(), pid.as_raw()));
        debug!(label = %node.label, pid = pid.as_raw(), "node incarnated");
    }

    let pipeline_id_for_flows = Ulid::new();
    // Si tap=true, creamos un FlowChannel por edge para el data plane.
    // Cada splitter pushea al sender del channel correspondiente.
    let pipeline_id = pipeline_id_for_flows;
    let mut flow_channels: Vec<crate::flow_channel::FlowChannel> = Vec::new();
    let mut splitter_channels: Vec<Vec<Option<crate::flow_channel::FlowSender>>> =
        Vec::with_capacity(splitter_specs.len());
    let mut edge_socket_for_splitter: Vec<Vec<Option<std::path::PathBuf>>> = Vec::new();
    for s in &splitter_specs {
        let mut senders_per_edge = Vec::with_capacity(s.edges.len());
        let mut paths_per_edge = Vec::with_capacity(s.edges.len());
        for (i, _em) in s.edges.iter().enumerate() {
            if !s.tap {
                senders_per_edge.push(None);
                paths_per_edge.push(None);
                continue;
            }
            // Socket name = pipeline_id full (26 chars ULID) + edge_idx.
            // ULID es único globalmente → cero colisiones entre runs.
            // Edge_idx desambigua múltiples sockets del mismo pipeline.
            // No incluimos from_label en el name (puede tener chars que
            // no van en paths Unix — los hints van en `EdgeDiscernment`).
            let id = format!("{}-{}", pipeline_id, i);
            let mut socket = crate::flow_channel::default_flow_socket_path(&id);
            // Fallback: si el path existe (raro — daemon crashed sin
            // cleanup), agregar suffix numérico hasta encontrar libre.
            let mut suffix = 1u32;
            while socket.exists() {
                let alt = format!("{id}-{suffix}");
                socket = crate::flow_channel::default_flow_socket_path(&alt);
                suffix += 1;
                if suffix > 1000 {
                    warn!(orig = id, "flow socket collision: 1000 retries — using as-is");
                    break;
                }
            }
            match crate::flow_channel::FlowChannel::with_replay_caps(
                socket.clone(),
                crate::flow_channel::ReplayCaps::new(spec.discern.replay_chunks, spec.discern.replay_bytes),
            ) {
                Ok(fc) => {
                    senders_per_edge.push(Some(fc.sender_handle()));
                    paths_per_edge.push(Some(socket));
                    flow_channels.push(fc);
                }
                Err(e) => {
                    warn!(?e, "flow channel new failed");
                    senders_per_edge.push(None);
                    paths_per_edge.push(None);
                }
            }
        }
        splitter_channels.push(senders_per_edge);
        edge_socket_for_splitter.push(paths_per_edge);
    }

    // Registramos los flow_channels en el manager AHORA, antes de await
    // las tasks. Esto permite que clientes externos hagan `flow list` y
    // se suscriban mientras el pipeline aún produce data.
    if let Some(mgr) = &manager {
        if !flow_channels.is_empty() {
            let drained: Vec<crate::flow_channel::FlowChannel> = flow_channels.drain(..).collect();
            mgr.retain_pipeline_flows(pipeline_id, drained).await;
        }
    }

    // Spawn mergers + splitters después del incarnate. Cada task posee
    // sus fds y los cierra al terminar (via Drop de OwnedFd).
    let mut merger_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    for m in merger_specs {
        merger_handles.push(spawn_merger(m));
    }
    let mut tap_handles: Vec<SplitterHandle> = Vec::new();
    for (s, senders) in splitter_specs.into_iter().zip(splitter_channels.into_iter()) {
        tap_handles.push(spawn_splitter(s, discerner.clone(), senders));
    }

    let mut edge_discernments = Vec::new();
    for (h, paths) in tap_handles.into_iter().zip(edge_socket_for_splitter.into_iter()) {
        match h.handle.await {
            Ok(eds) => {
                for (mut ed, path) in eds.into_iter().zip(paths.into_iter()) {
                    ed.flow_socket = path;
                    edge_discernments.push(ed);
                }
            }
            Err(e) => warn!(?e, "splitter handle joined with error"),
        }
    }
    for h in merger_handles {
        if let Err(e) = h.await {
            warn!(?e, "merger handle joined with error");
        }
    }

    Ok(PipelineLaunch {
        pipeline: pipeline_id,
        command_pids: pids,
        edge_discernments,
    })
}

#[allow(dead_code)]
fn short_ulid(u: &Ulid) -> String {
    let s = u.to_string();
    s[s.len() - 6..].to_string()
}

#[derive(Debug, Clone)]
struct EdgeMeta {
    from_label: String,
    from_output: String,
    to_label: String,
    to_input: String,
}

struct SplitterSpec {
    producer_r_fd: RawFd,
    consumer_w_fds: Vec<RawFd>,
    edges: Vec<EdgeMeta>,
    tap: bool,
    sample_bytes: usize,
    /// Rate-limit en bytes/s (0 = sin limit). Tras cada chunk de `n`
    /// bytes, splitter sleeps `n / max_bytes_per_sec` segundos.
    max_bytes_per_sec: u64,
}

struct SplitterHandle {
    handle: tokio::task::JoinHandle<Vec<EdgeDiscernment>>,
}

struct MergerSpec {
    producer_r_fds: Vec<RawFd>,
    consumer_w_fd: RawFd,
}

fn spawn_merger(spec: MergerSpec) -> tokio::task::JoinHandle<()> {
    for fd in &spec.producer_r_fds {
        set_nonblocking(*fd);
    }
    set_nonblocking(spec.consumer_w_fd);
    // Patrón: una task lectora por cada producer reenvía bytes a un mpsc.
    // El merger principal consume del mpsc y escribe al consumer.
    // Esto evita el "block en reader idle" del enfoque round-robin sobre
    // AsyncFd::ready() (los readers idle nunca dejan turno).
    tokio::spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
        let nr = spec.producer_r_fds.len();
        for fd in spec.producer_r_fds {
            let tx = tx.clone();
            tokio::spawn(async move {
                // SAFETY: ownership transferida.
                let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd_compat(fd) };
                let r = match AsyncFd::with_interest(owned, Interest::READABLE) {
                    Ok(a) => a,
                    Err(e) => {
                        warn!(?e, "merger reader AsyncFd");
                        return;
                    }
                };
                let mut buf = [0u8; 4096];
                loop {
                    match async_read(&r, &mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                // Drop de tx → cuando todos los readers cerraron, el rx
                // recibe None y el merger termina.
            });
        }
        drop(tx); // sólo los reader tasks tienen sus clones ahora.

        // SAFETY: ownership transferida al task.
        let w_owned = unsafe { std::os::fd::OwnedFd::from_raw_fd_compat(spec.consumer_w_fd) };
        let w = match AsyncFd::with_interest(w_owned, Interest::WRITABLE) {
            Ok(a) => a,
            Err(e) => {
                warn!(?e, "merger AsyncFd w");
                return;
            }
        };

        let mut total: u64 = 0;
        while let Some(chunk) = rx.recv().await {
            if async_write_all(&w, &chunk).await.is_err() {
                return;
            }
            total += chunk.len() as u64;
        }
        debug!(bytes = total, readers = nr, "merger finished");
    })
}

fn spawn_splitter(
    spec: SplitterSpec,
    discerner: Arc<DiscernPipeline>,
    edge_senders: Vec<Option<crate::flow_channel::FlowSender>>,
) -> SplitterHandle {
    set_nonblocking(spec.producer_r_fd);
    for fd in &spec.consumer_w_fds {
        set_nonblocking(*fd);
    }

    let handle = tokio::spawn(async move {
        // SAFETY: ownership transferida al task.
        let r_owned = unsafe { std::os::fd::OwnedFd::from_raw_fd_compat(spec.producer_r_fd) };
        let r = match AsyncFd::with_interest(r_owned, Interest::READABLE) {
            Ok(a) => a,
            Err(e) => {
                warn!(?e, "splitter AsyncFd r");
                return Vec::new();
            }
        };
        let mut writers: Vec<AsyncFd<std::os::fd::OwnedFd>> = Vec::with_capacity(spec.consumer_w_fds.len());
        for fd in spec.consumer_w_fds {
            let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd_compat(fd) };
            match AsyncFd::with_interest(owned, Interest::WRITABLE) {
                Ok(a) => writers.push(a),
                Err(e) => warn!(?e, "splitter AsyncFd w"),
            }
        }

        let mut sample: Vec<u8> = Vec::with_capacity(spec.sample_bytes);
        let mut buf = [0u8; 4096];
        let mut total: u64 = 0;
        let mut eof = false;
        let mut bucket = if spec.max_bytes_per_sec > 0 {
            Some(TokenBucket::new(spec.max_bytes_per_sec))
        } else {
            None
        };

        // Fase 1: sampling (sólo si tap=true) + replicación.
        while !eof && (spec.tap && sample.len() < spec.sample_bytes) {
            let n = match async_read(&r, &mut buf).await {
                Ok(0) => { eof = true; 0 }
                Ok(n) => n,
                Err(e) => { warn!(?e, "splitter read"); break; }
            };
            if n == 0 { break; }
            if spec.tap {
                let take = n.min(spec.sample_bytes - sample.len());
                sample.extend_from_slice(&buf[..take]);
            }
            // Token bucket: reserva ANTES de broadcast — si hay debt,
            // sleep antes de mandar al subscriber.
            if let Some(b) = bucket.as_mut() {
                let wait = b.reserve(n as u64);
                if !wait.is_zero() {
                    tokio::time::sleep(wait).await;
                }
            }
            broadcast_chunk(&writers, &edge_senders, &buf[..n]).await;
            total += n as u64;
        }

        let d = if spec.tap {
            discerner.discern(&sample, &Hint { path: None, size_total: None })
        } else {
            None
        };

        // Fase 2: replicación pura.
        while !eof {
            let n = match async_read(&r, &mut buf).await {
                Ok(0) => { eof = true; 0 }
                Ok(n) => n,
                Err(_) => break,
            };
            if n == 0 { break; }
            if let Some(b) = bucket.as_mut() {
                let wait = b.reserve(n as u64);
                if !wait.is_zero() {
                    tokio::time::sleep(wait).await;
                }
            }
            broadcast_chunk(&writers, &edge_senders, &buf[..n]).await;
            total += n as u64;
        }
        debug!(bytes = total, consumers = writers.len(), "splitter finished");

        // Mismo discernment para todos los edges del splitter (es el mismo
        // stream replicado). Devolvemos N entries (una por edge) para que
        // la UI/CLI los liste todos. flow_socket lo rellena el caller.
        spec.edges
            .into_iter()
            .map(|em| EdgeDiscernment {
                from_label: em.from_label,
                from_output: em.from_output,
                to_label: em.to_label,
                to_input: em.to_input,
                discernment: d.clone(),
                flow_socket: None,
            })
            .collect()
    });
    SplitterHandle { handle }
}

/// Token-bucket real con capacidad de burst.
/// - `rate_bps`: tokens (bytes) por segundo de refill.
/// - `capacity`: máx tokens acumulables. Default = 1 segundo de rate.
/// - `tokens`: tokens disponibles (puede negativos para "debt").
/// - `last_refill`: para calcular cuántos refill desde la última call.
struct TokenBucket {
    rate_bps: u64,
    capacity: u64,
    tokens: f64,
    last_refill: std::time::Instant,
}

impl TokenBucket {
    fn new(rate_bps: u64) -> Self {
        Self {
            rate_bps,
            capacity: rate_bps, // 1 second worth of burst.
            tokens: rate_bps as f64,
            last_refill: std::time::Instant::now(),
        }
    }

    /// Refill desde la última call según wall time. Reserva `cost`
    /// tokens; si no alcanza, retorna el sleep necesario.
    fn reserve(&mut self, cost: u64) -> std::time::Duration {
        let now = std::time::Instant::now();
        let elapsed_secs = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed_secs * self.rate_bps as f64)
            .min(self.capacity as f64);
        self.last_refill = now;

        self.tokens -= cost as f64;
        if self.tokens >= 0.0 {
            std::time::Duration::ZERO
        } else {
            // Debt: tiempo para recuperar a 0 tokens.
            let secs_needed = -self.tokens / self.rate_bps as f64;
            std::time::Duration::from_secs_f64(secs_needed)
        }
    }
}

async fn broadcast_chunk(
    writers: &[AsyncFd<std::os::fd::OwnedFd>],
    edge_senders: &[Option<crate::flow_channel::FlowSender>],
    data: &[u8],
) {
    // Internal pipes a los consumers del pipeline.
    for w in writers {
        let _ = async_write_all(w, data).await;
    }
    // Externos: broadcast a subscribers vía FlowChannel.
    // Cada edge tiene su propio sender (mismo data — el sample/discernment
    // viaja por broadcast separados para que un subscriber por edge vea su
    // stream específico).
    if edge_senders.iter().any(|s| s.is_some()) {
        let shared = std::sync::Arc::new(data.to_vec());
        for s in edge_senders {
            if let Some(s) = s {
                let _ = s.send(shared.clone());
            }
        }
    }
}

async fn async_read(
    afd: &AsyncFd<std::os::fd::OwnedFd>,
    buf: &mut [u8],
) -> std::io::Result<usize> {
    loop {
        let mut guard = afd.readable().await?;
        let fd = afd.as_raw_fd();
        // SAFETY: lectura sobre fd válido propiedad del AsyncFd.
        let r = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if r >= 0 {
            return Ok(r as usize);
        }
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::WouldBlock {
            guard.clear_ready();
            continue;
        }
        return Err(err);
    }
}

async fn async_write_all(
    afd: &AsyncFd<std::os::fd::OwnedFd>,
    mut buf: &[u8],
) -> std::io::Result<()> {
    while !buf.is_empty() {
        let mut guard = afd.writable().await?;
        let fd = afd.as_raw_fd();
        // SAFETY: escritura sobre fd válido propiedad del AsyncFd.
        let r = unsafe { libc::write(fd, buf.as_ptr() as *const _, buf.len()) };
        if r > 0 {
            buf = &buf[r as usize..];
            continue;
        }
        if r == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "write 0",
            ));
        }
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::WouldBlock {
            guard.clear_ready();
            continue;
        }
        return Err(err);
    }
    Ok(())
}

fn set_nonblocking(fd: RawFd) {
    // SAFETY: fcntl con F_SETFL es seguro para fds válidos.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

// Extension trait para abstraer la API de OwnedFd entre versiones (compat).
trait OwnedFdFromRawCompat: Sized {
    unsafe fn from_raw_fd_compat(fd: RawFd) -> Self;
}

impl OwnedFdFromRawCompat for std::os::fd::OwnedFd {
    unsafe fn from_raw_fd_compat(fd: RawFd) -> Self {
        use std::os::fd::FromRawFd;
        // SAFETY: el caller transfiere ownership de `fd` a la `OwnedFd`.
        unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) }
    }
}

// Re-export para que el unused warning del AsRawFd se calle si no se usa.
#[allow(dead_code)]
fn _keep_raw(_: &dyn AsRawFd) {}

#[cfg(test)]
mod tests {
    use super::*;
    use brahman_card::Payload;
    use arje_incarnate::IncarnatorConfig;
    use shuma_card::{CommandRef, DiscernPolicy, FlowEdge, PipelineSpec, WorkspaceId};

    fn cmd(label: &str, exec: &str, argv: &[&str]) -> CommandRef {
        CommandRef {
            label: label.into(),
            payload: Payload::Native {
                exec: exec.into(),
                argv: argv.iter().map(|s| s.to_string()).collect(),
                envp: vec![],
            },
            soma: Default::default(),
            flows: Default::default(),
            supervision: brahman_card::Supervision::OneShot,
        }
    }

    #[tokio::test]
    async fn pipeline_isolated_echo_to_cat_runs() {
        let spec = PipelineSpec {
            label: "echo-cat".into(),
            workspace: WorkspaceId::new(),
            nodes: vec![
                cmd("p1", "/bin/echo", &["hola pipeline aislado"]),
                cmd("p2", "/bin/cat", &[]),
            ],
            edges: vec![FlowEdge {
                from: 0,
                from_output: "stdout".into(),
                to: 1,
                to_input: "stdin".into(),
            }],
            discern: DiscernPolicy::default(),
            restart_on_failure: false,
            restart_backoff_ms: 200,
            restart_max_backoff_ms: 30_000,
            restart_max: 0,
        };
        let disc = Arc::new(DiscernPipeline::default_pipeline());
        let inc = Arc::new(Incarnator::new(IncarnatorConfig::default()));
        let launch = run_pipeline(&spec, "ws", false, disc, inc, None).await.unwrap();
        assert_eq!(launch.command_pids.len(), 2);
        // Cosecha.
        for (_, pid) in &launch.command_pids {
            let _ = nix::sys::wait::waitpid(nix::unistd::Pid::from_raw(*pid), None);
        }
    }

    #[tokio::test]
    async fn pipeline_fanin_two_to_one() {
        // 2 productores → 1 consumer (cat). El merger multiplexa.
        let spec = PipelineSpec {
            label: "fanin".into(),
            workspace: WorkspaceId::new(),
            nodes: vec![
                cmd("p1", "/bin/echo", &["from-p1"]),
                cmd("p2", "/bin/echo", &["from-p2"]),
                cmd("c", "/bin/cat", &[]),
            ],
            edges: vec![
                FlowEdge {
                    from: 0,
                    from_output: "stdout".into(),
                    to: 2,
                    to_input: "stdin".into(),
                },
                FlowEdge {
                    from: 1,
                    from_output: "stdout".into(),
                    to: 2,
                    to_input: "stdin".into(),
                },
            ],
            discern: DiscernPolicy::default(),
            restart_on_failure: false,
            restart_backoff_ms: 200,
            restart_max_backoff_ms: 30_000,
            restart_max: 0,
        };
        let disc = Arc::new(DiscernPipeline::default_pipeline());
        let inc = Arc::new(Incarnator::new(IncarnatorConfig::default()));
        let launch = run_pipeline(&spec, "ws", false, disc, inc, None).await.unwrap();
        assert_eq!(launch.command_pids.len(), 3);
        for (_, pid) in &launch.command_pids {
            let _ = nix::sys::wait::waitpid(nix::unistd::Pid::from_raw(*pid), None);
        }
    }

    #[tokio::test]
    async fn pipeline_fanout_one_to_two() {
        // 1 productor (echo) → 2 consumers (wc -c). Splitter replica.
        let spec = PipelineSpec {
            label: "fanout".into(),
            workspace: WorkspaceId::new(),
            nodes: vec![
                cmd("p", "/bin/echo", &["fanout-test"]),
                cmd("c1", "/bin/cat", &[]),
                cmd("c2", "/bin/cat", &[]),
            ],
            edges: vec![
                FlowEdge {
                    from: 0,
                    from_output: "stdout".into(),
                    to: 1,
                    to_input: "stdin".into(),
                },
                FlowEdge {
                    from: 0,
                    from_output: "stdout".into(),
                    to: 2,
                    to_input: "stdin".into(),
                },
            ],
            discern: DiscernPolicy::default(),
            restart_on_failure: false,
            restart_backoff_ms: 200,
            restart_max_backoff_ms: 30_000,
            restart_max: 0,
        };
        let disc = Arc::new(DiscernPipeline::default_pipeline());
        let inc = Arc::new(Incarnator::new(IncarnatorConfig::default()));
        let launch = run_pipeline(&spec, "ws", false, disc, inc, None).await.unwrap();
        assert_eq!(launch.command_pids.len(), 3);
        for (_, pid) in &launch.command_pids {
            let _ = nix::sys::wait::waitpid(nix::unistd::Pid::from_raw(*pid), None);
        }
    }

    #[tokio::test]
    async fn pipeline_isolated_with_tap_captures_discernment() {
        let spec = PipelineSpec {
            label: "json-cat".into(),
            workspace: WorkspaceId::new(),
            nodes: vec![
                cmd("p1", "/bin/echo", &["{\"hello\": 1}"]),
                cmd("p2", "/bin/cat", &[]),
            ],
            edges: vec![FlowEdge {
                from: 0,
                from_output: "stdout".into(),
                to: 1,
                to_input: "stdin".into(),
            }],
            discern: DiscernPolicy {
                sample_bytes: 4096,
                enrich_producer: true,
                replay_chunks: 32,
                replay_bytes: 0,
                max_bytes_per_sec: 0,
            },
            restart_on_failure: false,
            restart_backoff_ms: 200,
            restart_max_backoff_ms: 30_000,
            restart_max: 0,
        };
        let disc = Arc::new(DiscernPipeline::default_pipeline());
        let inc = Arc::new(Incarnator::new(IncarnatorConfig::default()));
        let launch = run_pipeline(&spec, "ws", true, disc, inc, None).await.unwrap();
        assert_eq!(launch.edge_discernments.len(), 1);
        let d = &launch.edge_discernments[0];
        let dis = d.discernment.as_ref().expect("discernment present");
        assert_eq!(dis.mime.as_deref(), Some("application/json"));
        // Cosecha.
        for (_, pid) in &launch.command_pids {
            let _ = nix::sys::wait::waitpid(nix::unistd::Pid::from_raw(*pid), None);
        }
    }
}
