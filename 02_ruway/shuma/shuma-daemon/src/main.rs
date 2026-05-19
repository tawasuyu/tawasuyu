//! `shuma-daemon` — punto de entrada del runtime de shuma.
//!
//! Responsabilidades:
//! - Escuchar el Unix socket admin (default: `$XDG_RUNTIME_DIR/shuma.sock`).
//! - Despachar mensajes del [`shuma_protocol`] al [`WorkspaceManager`].
//! - Reapear hijos periódicamente.
//!
//! Lo que NO hace en v1:
//! - Sidecar al broker / handshake con Init (futuro: cuando un workspace
//!   exponga `service_socket`, anunciar al broker).
//! - GUI (futuro `shuma-shell` con nahual_launcher).

use anyhow::Context;
use brahman_card::{Card, CardKind, Flow, Flows, Lifecycle, Payload, Supervision, TypeRef};
use ente_incarnate::IncarnatorConfig;
use shuma_core::WorkspaceManager;
use shuma_discern::{DiscernPipeline, Hint};
use shuma_protocol::{
    default_socket_path, read_frame, write_frame, CommandInfo as ProtoCommandInfo,
    EdgeDiscernmentInfo, FlowInfo, FlowThroughputInfo, QuotaReportInfo, Request, Response,
    WorkspaceStatsInfo, WorkspaceSummary,
};
use std::sync::Arc;
use tokio::net::{UnixListener, UnixStream};
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let sock = default_socket_path();
    if sock.exists() {
        // Si ya existe, asumimos restart limpio. Si hubiera otro daemon vivo,
        // bind fallaría con EADDRINUSE — más adelante: lockfile + check de PID.
        let _ = std::fs::remove_file(&sock);
    }
    if let Some(parent) = sock.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let listener = UnixListener::bind(&sock).with_context(|| format!("bind {}", sock.display()))?;
    info!(socket = %sock.display(), "shuma-daemon listening");
    let daemon_started = std::time::Instant::now();

    // Sidecar pool: una sesión global del daemon + N sesiones efímeras
    // por edge enriquecido tras cada pipeline tap.
    let sidecar_pool = match brahman_sidecar::SidecarPool::new() {
        Ok(p) => Some(Arc::new(p)),
        Err(e) => {
            warn!(?e, "SidecarPool falló — broker integration disabled");
            None
        }
    };
    if let Some(pool) = &sidecar_pool {
        pool.spawn(build_daemon_card(&sock));
    }

    let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig {
        // El daemon aún no se conecta al broker; cuando lo haga, este path
        // se llenará desde el handshake.
        bus_sock: None,
        notify_socket: None,
        extra_env: vec![("SHIPOTE_DAEMON".into(), "1".into())],
        // strict_caps=false en v1: queremos UX permisiva (correr en non-root
        // sin user_ns y avisar via warnings, no abortar).
        strict_caps: false,
    }));

    // Restaurar snapshot previo si existe. Workspaces se recrean; los
    // pids de comandos viejos NO se recuperan (kernel los mató). Los
    // pipelines vivos (con supervisor) se relanzan desde cero.
    let snapshot_path = shuma_core::persist::default_snapshot_path();
    let restore = match mgr.restore_snapshot(&snapshot_path).await {
        Ok(r) => r,
        Err(e) => {
            warn!(?e, "restore_snapshot falló — start fresh");
            shuma_core::persist::RestoreOutcome::default()
        }
    };
    // Relauncher de live_pipelines: como necesita inc+disc del daemon,
    // lo hacemos acá tras el restore. Cada uno mismo flujo que un run
    // normal — register_pipeline_commands + register_pipeline_supervisor.
    for entry in restore.live_pipelines {
        let inc = mgr.incarnator_handle();
        let disc = Arc::new(DiscernPipeline::default_pipeline());
        let workspace = entry.workspace;
        let ws_label = mgr.workspace_label(workspace).await.unwrap_or_default();
        let tap = entry.tap;
        let spec = entry.spec;
        match shuma_core::pipeline::run_pipeline(
            &spec, &ws_label, tap, disc, inc, Some(mgr.clone()),
        )
        .await
        {
            Ok(launch) => {
                mgr.register_pipeline_commands(workspace, launch.pipeline, launch.command_pids.clone()).await;
                mgr.register_pipeline_supervisor(launch.pipeline, workspace, spec, tap).await;
                info!(label = %launch.pipeline, "live pipeline relaunched from snapshot");
            }
            Err(e) => warn!(?e, "live pipeline relaunch failed"),
        }
    }

    // Shutdown handler: SIGTERM/SIGINT → drain (stop_with_grace de todos
    // los workspaces) → snapshot → exit. El drain usa grace=1s para dar
    // chance a los comandos a terminar limpio antes del SIGKILL.
    {
        let mgr = mgr.clone();
        let path = snapshot_path.clone();
        tokio::spawn(async move {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("SIGTERM handler");
            let mut int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
                .expect("SIGINT handler");
            let sig_name = tokio::select! {
                _ = term.recv() => "SIGTERM",
                _ = int.recv() => "SIGINT",
            };
            info!(signal = sig_name, "shuma-daemon shutdown: draining workspaces");

            // 1) Snapshot ANTES del drain — preserva intención declarada
            //    (los workspace specs siguen vivos en el snapshot aunque
            //    matemos los procesos hijos).
            if let Err(e) = mgr.save_snapshot(&path).await {
                warn!(?e, "save_snapshot falló");
            }

            // 2) Drain: stop_with_grace de todos los workspaces vivos.
            //    Grace 1s da chance a apps Type=notify de hacer cleanup.
            let workspaces = mgr.list().await;
            let n = workspaces.len();
            for ws in workspaces {
                if let Err(e) = mgr
                    .stop_with_grace(ws.id, std::time::Duration::from_millis(1000))
                    .await
                {
                    warn!(?e, %ws.id, "stop_with_grace falló en drain");
                }
            }
            info!(drained = n, "drain complete");
            std::process::exit(0);
        });
    }

    let discerner = Arc::new(DiscernPipeline::default_pipeline());

    // Reaper periódico cada 500 ms. Además drena pipelines pendientes
    // de restart (supervisión a nivel pipeline).
    {
        let mgr = mgr.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(500));
            loop {
                tick.tick().await;
                mgr.reap_dead().await;
                let pending = mgr.take_pending_restarts().await;
                for sup in pending {
                    let backoff = std::time::Duration::from_millis(sup.current_backoff_ms);
                    info!(
                        label = %sup.spec.label,
                        restart_count = sup.restart_count,
                        backoff_ms = sup.current_backoff_ms,
                        "pipeline restart: relaunching after backoff"
                    );
                    // Backoff antes del relaunch — anti-thrash.
                    tokio::time::sleep(backoff).await;
                    let inc = mgr.incarnator_handle();
                    let disc = std::sync::Arc::new(DiscernPipeline::default_pipeline());
                    let workspace = sup.spec.workspace;
                    let ws_label = mgr.workspace_label(workspace).await.unwrap_or_default();
                    let tap = sup.tap;
                    let mut new_spec = sup.spec.clone();
                    new_spec.restart_on_failure = true;
                    // Escalar el backoff para la PRÓXIMA falla.
                    let next_backoff = (sup.current_backoff_ms * 2)
                        .min(new_spec.restart_max_backoff_ms);
                    match shuma_core::pipeline::run_pipeline(
                        &new_spec,
                        &ws_label,
                        tap,
                        disc,
                        inc,
                        Some(mgr.clone()),
                    )
                    .await
                    {
                        Ok(launch) => {
                            mgr.register_pipeline_commands(
                                workspace,
                                launch.pipeline,
                                launch.command_pids.clone(),
                            )
                            .await;
                            // Re-registrar supervisor con backoff escalado +
                            // restart_count preservado.
                            mgr.register_pipeline_supervisor_with_state(
                                launch.pipeline,
                                workspace,
                                new_spec,
                                tap,
                                sup.restart_count,
                                next_backoff,
                            )
                            .await;
                        }
                        Err(e) => {
                            warn!(?e, "pipeline restart failed");
                        }
                    }
                }
            }
        });
    }

    // UID propio (para auth). SHIPOTE_TRUST_ANYONE=1 deshabilita.
    let own_uid = nix::unistd::getuid().as_raw();
    let trust_anyone = std::env::var("SHIPOTE_TRUST_ANYONE").as_deref() == Ok("1");
    if trust_anyone {
        warn!("SHIPOTE_TRUST_ANYONE=1 — accepting any peer uid");
    }

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                // Auth: SO_PEERCRED es automático en Unix sockets. Si
                // el uid del peer no coincide con el nuestro, rechazo
                // antes de procesar nada (a menos que esté permitido).
                if !trust_anyone {
                    match peer_uid(&stream) {
                        Ok(peer) if peer == own_uid => {}
                        Ok(peer) => {
                            warn!(peer, own = own_uid, "rejecting peer with different uid");
                            drop(stream);
                            continue;
                        }
                        Err(e) => {
                            warn!(?e, "could not read peer uid — rejecting");
                            drop(stream);
                            continue;
                        }
                    }
                }
                let mgr = mgr.clone();
                let disc = discerner.clone();
                let pool = sidecar_pool.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, mgr, disc, pool, daemon_started).await {
                        warn!(?e, "client handler error");
                    }
                });
            }
            Err(e) => {
                error!(?e, "accept failed");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

/// Lee SO_PEERCRED del Unix socket conectado. Devuelve el uid del peer.
fn peer_uid(stream: &tokio::net::UnixStream) -> std::io::Result<u32> {
    use std::os::fd::AsRawFd;
    let fd = stream.as_raw_fd();
    let mut ucred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let r = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut ucred as *mut _ as *mut _,
            &mut len,
        )
    };
    if r != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(ucred.uid)
}

async fn handle_client(
    mut stream: UnixStream,
    mgr: Arc<WorkspaceManager>,
    disc: Arc<DiscernPipeline>,
    pool: Option<Arc<brahman_sidecar::SidecarPool>>,
    daemon_started: std::time::Instant,
) -> anyhow::Result<()> {
    // Audit: peer uid lo leemos una vez aquí (no cambia durante la conexión).
    let peer = peer_uid(&stream).unwrap_or(u32::MAX);
    loop {
        let req: Request = match read_frame(&mut stream).await {
            Ok(r) => r,
            Err(shuma_protocol::ProtocolError::Closed) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        audit_request(peer, &req);
        let resp = dispatch(&mgr, &disc, &pool, daemon_started, req).await;
        write_frame(&mut stream, &resp).await?;
    }
}

/// Path canónico del audit log: `$XDG_STATE_HOME/shuma/audit.log` o
/// fallback `$HOME/.local/state/shuma/audit.log`.
fn default_audit_log_path() -> std::path::PathBuf {
    if let Ok(state) = std::env::var("XDG_STATE_HOME") {
        return std::path::PathBuf::from(state).join("shuma/audit.log");
    }
    if let Ok(home) = std::env::var("HOME") {
        return std::path::PathBuf::from(home).join(".local/state/shuma/audit.log");
    }
    std::path::PathBuf::from("/tmp/shuma-audit.log")
}

/// Cap del audit log antes de rotar a `audit.log.1`. 1 MiB.
const AUDIT_LOG_MAX_BYTES: u64 = 1 << 20;

/// Append + rotate (mueve a `.1` si supera el cap). Append-only, sin
/// reordenar. Sync: cada line, fsync no — el log es defensive, no
/// transactional.
fn append_audit_line(path: &std::path::Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    // Rotar si pasa el cap.
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() >= AUDIT_LOG_MAX_BYTES {
            let rotated = path.with_extension("log.1");
            let _ = std::fs::rename(path, &rotated);
        }
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")?;
    Ok(())
}

/// Loguea cada mutación con target="audit" y el peer uid. Reads (ping,
/// list, stats) se omiten para no inundar el log.
fn audit_request(peer_uid: u32, req: &Request) {
    let (action, detail) = match req {
        Request::WorkspaceCreate { spec } => ("workspace.create", format!("label={}", spec.label)),
        Request::WorkspaceStop { id, grace_ms } => ("workspace.stop", format!("id={id} grace_ms={grace_ms}")),
        Request::Run { workspace, exec, restart_on_failure, .. } => (
            "run",
            format!("ws={workspace} exec={exec} restart={restart_on_failure}"),
        ),
        Request::PipelineRun { spec, tap, .. } => ("pipeline.run", format!("label={} tap={tap}", spec.label)),
        Request::PipelineRunSaved { name, tap, .. } => ("pipeline.run-saved", format!("name={name} tap={tap}")),
        Request::PipelineStop { pipeline, grace_ms } => ("pipeline.stop", format!("id={pipeline} grace_ms={grace_ms}")),
        Request::PipelineSave { name, .. } => ("pipeline.save", format!("name={name}")),
        Request::PipelineDrop { name } => ("pipeline.drop", format!("name={name}")),
        Request::FlowDrop { pipeline } => ("flow.drop", format!("pipeline={pipeline}")),
        // Reads (no audit):
        Request::Ping
        | Request::Health
        | Request::WorkspaceList
        | Request::WorkspaceStats { .. }
        | Request::WorkspaceQuota { .. }
        | Request::WorkspaceStatsHistory { .. }
        | Request::WorkspaceFullSummary { .. }
        | Request::CommandList { .. }
        | Request::CommandLogs { .. }
        | Request::PipelineSavedList
        | Request::FlowList
        | Request::FlowThroughput
        | Request::Discern { .. }
        | Request::Capabilities => return,
    };
    info!(target: "audit", uid = peer_uid, action, detail = %detail, "audit");
    // Append a file. Failure no es fatal — sólo se pierde la entry.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let line = format!("ts={ts} uid={peer_uid} action={action} {detail}");
    let path = default_audit_log_path();
    if let Err(e) = append_audit_line(&path, &line) {
        // Sólo loguear si el filesystem está roto. No reportar al cliente.
        tracing::debug!(?e, path = %path.display(), "audit file write failed");
    }
}

async fn dispatch(
    mgr: &Arc<WorkspaceManager>,
    disc: &DiscernPipeline,
    pool: &Option<Arc<brahman_sidecar::SidecarPool>>,
    daemon_started: std::time::Instant,
    req: Request,
) -> Response {
    match req {
        Request::Ping => Response::Pong,

        Request::Health => {
            let counts = mgr.health_counts().await;
            Response::Health {
                version: env!("CARGO_PKG_VERSION").to_string(),
                uptime_ms: daemon_started.elapsed().as_millis() as u64,
                alive_workspaces: counts.alive_workspaces,
                alive_commands: counts.alive_commands,
                alive_pipelines: counts.alive_pipelines,
                active_flows: counts.active_flows,
                dirty: mgr.is_dirty(),
            }
        }

        Request::WorkspaceCreate { spec } => match mgr.create(spec).await {
            Ok((id, warnings)) => Response::WorkspaceCreated { id, warnings },
            Err(e) => Response::Error { message: format!("{e}") },
        },

        Request::WorkspaceList => {
            let items = mgr
                .list()
                .await
                .into_iter()
                .map(|s| WorkspaceSummary {
                    id: s.id,
                    label: s.label,
                    commands: s.commands,
                    uptime_ms: s.uptime_ms,
                })
                .collect();
            Response::WorkspaceList { items }
        }

        Request::WorkspaceStop { id, grace_ms } => {
            match mgr
                .stop_with_grace(id, std::time::Duration::from_millis(grace_ms))
                .await
            {
                Ok(reaped) => Response::WorkspaceStopped { id, reaped },
                Err(e) => Response::Error { message: format!("{e}") },
            }
        }

        Request::Run { workspace, exec, argv, envp, restart_on_failure } => {
            match mgr
                .run_with_options(workspace, exec, argv, envp, restart_on_failure)
                .await
            {
                Ok(s) => Response::RunStarted {
                    workspace,
                    command_id: s.id,
                    pid: s.pid,
                },
                Err(e) => Response::Error { message: format!("{e}") },
            }
        }

        Request::PipelineRun { spec, tap, vars } => {
            let vars_map: std::collections::HashMap<String, String> = vars.into_iter().collect();
            let spec = match shuma_card::substitute_vars(&spec, &vars_map) {
                Ok(s) => s,
                Err(e) => return Response::Error { message: format!("template: {e}") },
            };
            let disc = DiscernPipeline::default_pipeline();
            let inc = mgr.incarnator_handle();
            let ws_label = mgr.workspace_label(spec.workspace).await.unwrap_or_default();
            match shuma_core::pipeline::run_pipeline(
                &spec,
                &ws_label,
                tap,
                std::sync::Arc::new(disc),
                inc,
                Some(mgr.clone()),
            )
            .await
            {
                Ok(launch) => {
                    let pipeline_id = launch.pipeline;
                    announce_edges_to_broker(pool.as_deref(), &pipeline_id, &launch.edge_discernments);
                    let cmds = launch.command_pids;
                    mgr.register_pipeline_commands(spec.workspace, pipeline_id, cmds.clone()).await;
                    mgr.register_pipeline_supervisor(pipeline_id, spec.workspace, spec.clone(), tap).await;
                    let edges = launch.edge_discernments.into_iter().map(map_edge_to_info).collect();
                    Response::PipelineStarted {
                        pipeline: pipeline_id,
                        command_pids: cmds,
                        edges,
                    }
                }
                Err(e) => Response::Error { message: format!("{e}") },
            }
        }

        Request::Discern { sample, hint_path } => {
            let path_str = hint_path.as_ref().and_then(|p| p.to_str());
            let hint = Hint {
                path: path_str,
                size_total: None,
            };
            match disc.discern(&sample, &hint) {
                Some(d) => Response::Discernment {
                    ty: format!("{:?}", d.ty),
                    confidence: d.confidence,
                    mime: d.mime,
                    lens: d.lens,
                },
                None => Response::Error { message: "no discernment".into() },
            }
        }

        Request::CommandList { workspace } => {
            let items: Vec<ProtoCommandInfo> = mgr
                .list_commands(workspace)
                .await
                .into_iter()
                .map(|c| ProtoCommandInfo {
                    id: c.id,
                    label: c.label,
                    pid: c.pid,
                    alive: c.alive,
                    exit_status: c.exit_status,
                    log_bytes: c.log_bytes,
                })
                .collect();
            Response::CommandList { items }
        }

        Request::CommandLogs { workspace, command, tail_bytes, stream } => {
            let s = match stream.as_str() {
                "stdout" => shuma_core::LogStream::Stdout,
                "stderr" => shuma_core::LogStream::Stderr,
                _ => shuma_core::LogStream::Both,
            };
            match mgr.get_command_logs(workspace, command, tail_bytes, s).await {
                Some(bytes) => Response::CommandLogs { bytes },
                None => Response::Error {
                    message: format!("no logs for command {command} in workspace {workspace}"),
                },
            }
        }

        Request::PipelineSave { name, spec } => {
            mgr.save_pipeline(name.clone(), spec).await;
            Response::PipelineSaved { name }
        }

        Request::PipelineSavedList => {
            let names = mgr.list_saved_pipelines().await;
            Response::PipelineSavedList { names }
        }

        Request::PipelineDrop { name } => {
            let existed = mgr.drop_saved_pipeline(&name).await;
            Response::PipelineDropped { name, existed }
        }

        Request::PipelineRunSaved { name, tap, vars } => match mgr.get_saved_pipeline(&name).await {
            Some(spec) => {
                let vars_map: std::collections::HashMap<String, String> = vars.into_iter().collect();
                let spec = match shuma_card::substitute_vars(&spec, &vars_map) {
                    Ok(s) => s,
                    Err(e) => return Response::Error { message: format!("template: {e}") },
                };
                let disc = DiscernPipeline::default_pipeline();
                let inc = mgr.incarnator_handle();
                let ws_label = mgr.workspace_label(spec.workspace).await.unwrap_or_default();
                match shuma_core::pipeline::run_pipeline(
                    &spec,
                    &ws_label,
                    tap,
                    std::sync::Arc::new(disc),
                    inc,
                    Some(mgr.clone()),
                )
                .await
                {
                    Ok(launch) => {
                        let pipeline_id = launch.pipeline;
                        announce_edges_to_broker(pool.as_deref(), &pipeline_id, &launch.edge_discernments);
                        let cmds = launch.command_pids;
                        mgr.register_pipeline_commands(spec.workspace, pipeline_id, cmds.clone()).await;
                        mgr.register_pipeline_supervisor(pipeline_id, spec.workspace, spec.clone(), tap).await;
                        let edges = launch.edge_discernments.into_iter().map(map_edge_to_info).collect();
                        Response::PipelineStarted {
                            pipeline: pipeline_id,
                            command_pids: cmds,
                            edges,
                        }
                    }
                    Err(e) => Response::Error { message: format!("{e}") },
                }
            }
            None => Response::Error {
                message: format!("pipeline `{name}` no encontrado"),
            },
        },

        Request::PipelineStop { pipeline, grace_ms } => {
            let reaped = mgr
                .stop_pipeline(pipeline, std::time::Duration::from_millis(grace_ms))
                .await;
            Response::PipelineStopped { pipeline, reaped }
        }

        Request::WorkspaceStats { workspace } => match mgr.workspace_stats(workspace).await {
            Some(s) => Response::WorkspaceStats {
                info: WorkspaceStatsInfo {
                    commands_alive: s.commands_alive,
                    commands_total: s.commands_total,
                    rss_bytes: s.rss_bytes,
                    rss_peak_bytes: s.rss_peak_bytes,
                    cpu_usec: s.cpu_usec,
                    cpu_percent: s.cpu_percent,
                    cpu_cores: s.cpu_cores,
                    source: s.source,
                    uptime_ms: s.uptime_ms,
                },
            },
            None => Response::Error {
                message: format!("workspace {workspace} not found"),
            },
        },

        Request::WorkspaceStatsHistory { workspace, tail } => {
            match mgr.workspace_stats_history(workspace, tail).await {
                Some(samples) => {
                    let mapped: Vec<WorkspaceStatsInfo> = samples
                        .into_iter()
                        .map(|s| WorkspaceStatsInfo {
                            commands_alive: s.commands_alive,
                            commands_total: s.commands_total,
                            rss_bytes: s.rss_bytes,
                            rss_peak_bytes: s.rss_peak_bytes,
                            cpu_usec: s.cpu_usec,
                            cpu_percent: s.cpu_percent,
                            cpu_cores: s.cpu_cores,
                            source: s.source,
                            uptime_ms: s.uptime_ms,
                        })
                        .collect();
                    Response::WorkspaceStatsHistory { samples: mapped }
                }
                None => Response::Error {
                    message: format!("workspace {workspace} not found"),
                },
            }
        }

        Request::WorkspaceFullSummary { workspace } => {
            let stats = match mgr.workspace_stats(workspace).await {
                Some(s) => WorkspaceStatsInfo {
                    commands_alive: s.commands_alive,
                    commands_total: s.commands_total,
                    rss_bytes: s.rss_bytes,
                    rss_peak_bytes: s.rss_peak_bytes,
                    cpu_usec: s.cpu_usec,
                    cpu_percent: s.cpu_percent,
                    cpu_cores: s.cpu_cores,
                    source: s.source,
                    uptime_ms: s.uptime_ms,
                },
                None => return Response::Error { message: format!("workspace {workspace} not found") },
            };
            let quota = match mgr.workspace_quota(workspace).await {
                Some(q) => QuotaReportInfo {
                    mem_limit: q.mem_limit,
                    nproc_limit: q.nproc_limit,
                    breaches: q.breaches,
                },
                None => QuotaReportInfo { mem_limit: None, nproc_limit: None, breaches: Vec::new() },
            };
            let commands = mgr
                .list_commands(workspace)
                .await
                .into_iter()
                .map(|c| ProtoCommandInfo {
                    id: c.id,
                    label: c.label,
                    pid: c.pid,
                    alive: c.alive,
                    exit_status: c.exit_status,
                    log_bytes: c.log_bytes,
                })
                .collect();
            // Flow sockets de pipelines whose workspace == este.
            let flow_sockets = mgr
                .list_flow_pipelines()
                .await
                .into_iter()
                .flat_map(|(_, sockets)| sockets)
                .collect();
            Response::WorkspaceFullSummary { stats, quota, commands, flow_sockets }
        }

        Request::WorkspaceQuota { workspace } => match mgr.workspace_quota(workspace).await {
            Some(q) => Response::WorkspaceQuota {
                info: QuotaReportInfo {
                    mem_limit: q.mem_limit,
                    nproc_limit: q.nproc_limit,
                    breaches: q.breaches,
                },
            },
            None => Response::Error {
                message: format!("workspace {workspace} not found"),
            },
        },

        Request::FlowList => {
            let items = mgr
                .list_flow_pipelines()
                .await
                .into_iter()
                .map(|(pipeline, sockets)| FlowInfo { pipeline, sockets })
                .collect();
            Response::FlowList { items }
        }

        Request::FlowThroughput => {
            let items = mgr
                .flow_throughput()
                .await
                .into_iter()
                .map(|(socket, bytes_total, bytes_per_sec)| FlowThroughputInfo {
                    socket,
                    bytes_total,
                    bytes_per_sec,
                })
                .collect();
            Response::FlowThroughput { items }
        }

        Request::FlowDrop { pipeline } => {
            let existed = mgr.drop_pipeline_flows(pipeline).await;
            Response::FlowDropped { pipeline, existed }
        }

        Request::Capabilities => {
            let c = mgr.incarnator().capabilities();
            Response::Capabilities {
                kernel_version: c.kernel_version,
                user_ns: format!("{:?}", c.user_ns),
                cgroup_v2: format!("{:?}", c.cgroup_v2),
                cgroup_delegated: c.cgroup_delegated,
                has_cap_sys_admin: c.has_cap_sys_admin,
            }
        }
    }
}

fn map_edge_to_info(e: shuma_core::pipeline::EdgeDiscernment) -> EdgeDiscernmentInfo {
    EdgeDiscernmentInfo {
        from_label: e.from_label,
        from_output: e.from_output,
        to_label: e.to_label,
        to_input: e.to_input,
        ty: e.discernment.as_ref().map(|d| format!("{:?}", d.ty)),
        mime: e.discernment.as_ref().and_then(|d| d.mime.clone()),
        lens: e.discernment.as_ref().and_then(|d| d.lens.clone()),
        confidence: e.discernment.as_ref().map(|d| d.confidence).unwrap_or(0.0),
        flow_socket: e.flow_socket,
    }
}

/// Por cada edge con TypeRef detectado, spawneamos una Card efímera en el
/// SidecarPool que se anuncia al broker como producer del TypeRef
/// enriquecido. Esto permite a otros explorers (broker-explorer, etc.)
/// ver que shuma vio JSON/text/wasm/etc. saliendo de un pipeline.
fn announce_edges_to_broker(
    pool: Option<&brahman_sidecar::SidecarPool>,
    pipeline: &ulid::Ulid,
    edges: &[shuma_core::pipeline::EdgeDiscernment],
) {
    let Some(pool) = pool else { return };
    for e in edges {
        let Some(d) = &e.discernment else { continue };
        let label = format!(
            "shuma.flow.{}.{}.{}.{}",
            short_ulid(pipeline),
            e.from_label,
            e.from_output,
            type_label(&d.ty)
        );
        let mut card = Card::new(label);
        card.kind = CardKind::Data;
        card.lifecycle = Lifecycle::Oneshot;
        card.payload = Payload::Virtual;
        card.supervision = Supervision::OneShot;
        card.flow = Flows {
            input: Vec::new(),
            output: vec![Flow {
                name: e.from_output.clone(),
                ty: d.ty.clone(),
                pin_to: None,
            }],
        };
        pool.spawn(card);
        info!(pipeline = %pipeline, from = %e.from_label, ty = ?d.ty, "edge announced to broker");
    }
}

fn short_ulid(u: &ulid::Ulid) -> String {
    let s = u.to_string();
    s[s.len() - 6..].to_string()
}

fn type_label(t: &TypeRef) -> String {
    match t {
        TypeRef::Primitive { name } => name.clone(),
        TypeRef::Wit { package, name, .. } => format!("{package}.{name}"),
    }
}

/// Card del daemon. La presentamos al broker así otras sesiones pueden
/// descubrir que shuma está corriendo y, eventualmente, conectarse
/// como consumidoras del flow `workspaces` (futuro: que la GUI o el
/// broker-explorer los listen vía broker en lugar de socket directo).
fn build_daemon_card(service_socket: &std::path::Path) -> Card {
    let mut card = Card::new("shuma.daemon");
    card.kind = CardKind::Ente;
    card.lifecycle = Lifecycle::Daemon;
    card.payload = Payload::Virtual; // el daemon ya está corriendo (no es PID 1 quien lo encarna)
    card.supervision = Supervision::Delegate;
    card.service_socket = Some(service_socket.to_path_buf());
    card.flow = Flows {
        input: Vec::new(),
        output: vec![
            Flow {
                name: "workspaces".into(),
                ty: TypeRef::Wit {
                    package: "shuma:admin".into(),
                    interface: None,
                    name: "workspace-list".into(),
                },
                pin_to: None,
            },
            Flow {
                name: "discern".into(),
                ty: TypeRef::Wit {
                    package: "shuma:admin".into(),
                    interface: None,
                    name: "discernment".into(),
                },
                pin_to: None,
            },
        ],
    };
    card
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("SHIPOTE_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).init();
}
