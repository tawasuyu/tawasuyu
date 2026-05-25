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
use card_core::{Card, CardKind, Flow, Flows, Lifecycle, Payload, Supervision, TypeRef};
use arje_incarnate::IncarnatorConfig;
use shuma_core::WorkspaceManager;
use shuma_discern::{DiscernPipeline, Hint};
use shuma_link::{FramedChannel, KnownPeers};
use shuma_protocol::{
    default_socket_path, read_frame, write_frame, CommandInfo as ProtoCommandInfo,
    EdgeDiscernmentInfo, ExecKind as ProtoExecKind, FlowInfo, FlowThroughputInfo, QuotaReportInfo,
    Request, Response, WorkspaceStatsInfo, WorkspaceSummary,
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
    let sidecar_pool = match card_sidecar::SidecarPool::new() {
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

    // Listener TCP autenticado opt-in: `SHUMA_LISTEN_TCP=host:port` lo
    // activa. Cada conexión hace handshake Noise XK con la identidad
    // del daemon (auto-generada y persistida en `~/.config/shuma/keys/
    // identity.x25519`) y valida la pubkey del cliente contra la
    // allowlist `~/.config/shuma/known_peers.txt`. Es la base para que
    // shuma-remote-exec hable con un daemon remoto sin SSH externo.
    if let Ok(addr) = std::env::var("SHUMA_LISTEN_TCP") {
        let kp_path = shuma_link::Keypair::default_path()
            .context("no se puede determinar el directorio de configuración (XDG)")?;
        let keypair = shuma_link::Keypair::load_or_generate(&kp_path)
            .context("identity keypair")?;
        let peers_path = KnownPeers::default_path()
            .context("no se puede determinar el directorio de configuración (XDG)")?;
        let tcp_listener = tokio::net::TcpListener::bind(&addr)
            .await
            .with_context(|| format!("bind TCP {addr}"))?;
        info!(
            socket = %addr,
            identity = %keypair.public().to_hex(),
            "shuma-daemon TCP listener up (Noise XK + KnownPeers)",
        );
        let mgr_tcp = mgr.clone();
        let disc_tcp = discerner.clone();
        let pool_tcp = sidecar_pool.clone();
        let daemon_started_tcp = daemon_started;
        tokio::spawn(async move {
            loop {
                match tcp_listener.accept().await {
                    Ok((tcp, remote_addr)) => {
                        // Re-cargamos peers en cada accept — barato, y
                        // permite editar el allowlist sin reiniciar.
                        let peers = KnownPeers::load(&peers_path).unwrap_or_default();
                        let our_kp = keypair.clone();
                        let mgr = mgr_tcp.clone();
                        let disc = disc_tcp.clone();
                        let pool = pool_tcp.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_enc_client(
                                tcp,
                                our_kp,
                                peers,
                                mgr,
                                disc,
                                pool,
                                daemon_started_tcp,
                            )
                            .await
                            {
                                warn!(?e, %remote_addr, "encrypted client handler error");
                            }
                        });
                    }
                    Err(e) => {
                        error!(?e, "TCP accept failed");
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            }
        });
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
    pool: Option<Arc<card_sidecar::SidecarPool>>,
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

        // El subprotocolo `ExecStream` produce N frames sobre la misma
        // conexión hasta un terminal. Lo manejamos inline en vez de pasar
        // por `dispatch` (que es request/response 1:1).
        if let Request::ExecStream { cwd, exec, capture_limit_bytes, stdin_data } = req {
            handle_exec_stream(&mut stream, cwd, exec, capture_limit_bytes, stdin_data).await?;
            continue;
        }

        let resp = dispatch(&mgr, &disc, &pool, daemon_started, req).await;
        write_frame(&mut stream, &resp).await?;
    }
}

/// Subprotocolo ExecStream: spawnea con `shuma-exec` y reemite cada
/// evento como un frame de `Response::Exec*` sobre `stream`. Cuando el
/// cliente cierra la conexión a mitad de stream, detectamos el error de
/// escritura y matamos el proceso — convención SSH/PTY.
async fn handle_exec_stream(
    stream: &mut UnixStream,
    cwd: String,
    exec: ProtoExecKind,
    capture_limit_bytes: usize,
    stdin_data: Option<String>,
) -> anyhow::Result<()> {
    let exec = match exec {
        ProtoExecKind::Shell { line, program } => shuma_exec::Exec::Shell { line, program },
        ProtoExecKind::Direct { stages } => shuma_exec::Exec::Direct {
            stages: stages
                .into_iter()
                .map(|s| shuma_exec::StageSpec { program: s.program, args: s.args })
                .collect(),
        },
    };
    let spec = shuma_exec::CommandSpec {
        exec,
        cwd,
        capture_limit: capture_limit_bytes,
        spill_path: None, // el cliente no expone path local del daemon
        stdin_data,
    };
    let mut handle = shuma_exec::run(&spec);
    // Capturamos el "Killer" antes de mover el RunHandle al hilo bridge —
    // así podemos disparar SIGKILL desde la tarea async sin contender el
    // lock del lector (`next_event` puede estar bloqueado en `rx.recv`).
    let killer = handle.killer();

    // Bridge sync→async: un hilo dedicado bloquea en `next_event()` y
    // reenvía por un canal tokio.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<shuma_exec::RunEvent>();
    let _bridge = std::thread::spawn(move || {
        while let Some(ev) = handle.next_event() {
            if tx.send(ev).is_err() {
                return;
            }
        }
    });

    // Loop dual: por un lado los eventos del proceso (rx), por otro el
    // socket del cliente. Cualquier byte o EOF en el socket significa
    // "abort" (el cliente no debe escribir mientras dura el stream).
    // Sin este watcher, un proceso silencioso (p.ej. `sleep 30`) deja
    // colgado al handler hasta que el cliente cae por timeout TCP.
    let mut byte = [0u8; 1];
    loop {
        tokio::select! {
            biased;
            ev = rx.recv() => {
                let Some(ev) = ev else { break };
                let terminal = matches!(
                    ev,
                    shuma_exec::RunEvent::Exited(_) | shuma_exec::RunEvent::Failed(_)
                );
                let resp = exec_event_to_response(ev);
                if let Err(e) = write_frame(stream, &resp).await {
                    killer.kill();
                    return Err(e.into());
                }
                if terminal {
                    break;
                }
            }
            // `read_exact` es cancel-safe en tokio: si se cancela la rama
            // no se pierden bytes. Aquí cualquier resultado (Ok = byte
            // inesperado, Err = EOF/error) cuenta como señal de abort.
            res = tokio::io::AsyncReadExt::read_exact(stream, &mut byte) => {
                let _ = res; // ambos sentidos significan "kill"
                killer.kill();
                // Drenamos lo que quede en cola hasta el terminal para
                // que el bridge cierre limpio (el thread del lector saldrá
                // solo cuando el proceso muera y los readers vean EOF).
                while let Some(ev) = rx.recv().await {
                    if matches!(ev, shuma_exec::RunEvent::Exited(_) | shuma_exec::RunEvent::Failed(_)) {
                        break;
                    }
                }
                break;
            }
        }
    }
    Ok(())
}

fn exec_event_to_response(ev: shuma_exec::RunEvent) -> Response {
    match ev {
        shuma_exec::RunEvent::Stdout(l) => Response::ExecStdout(l),
        shuma_exec::RunEvent::Stderr(l) => Response::ExecStderr(l),
        shuma_exec::RunEvent::Truncated => Response::ExecTruncated,
        shuma_exec::RunEvent::Spilled(p) => Response::ExecSpilled(p),
        shuma_exec::RunEvent::Exited(c) => Response::ExecExited(c),
        shuma_exec::RunEvent::Failed(m) => Response::ExecFailed(m),
        // PTY bytes no se reemiten por ExecStream (el protocolo es
        // unidireccional). El subprotocolo nunca debería verlos porque
        // la request `ExecStream` traduce a `Exec::Direct/Shell`, no
        // `Exec::Pty`. Si llegan, los reportamos como un fallo para no
        // perderlos silenciosamente.
        shuma_exec::RunEvent::Bytes(_) => Response::ExecFailed(
            "PTY bytes inesperados en ExecStream — el daemon no debió spawnear PTY aquí".into(),
        ),
    }
}

/// Atiende una conexión TCP autenticada por Noise XK.
///
/// Flujo:
/// 1. Handshake server: descubre la pubkey del cliente.
/// 2. Verifica que esté en `peers` (allowlist). Si no, log + drop.
/// 3. Loop dispatch idéntico a `handle_client` pero sobre
///    `FramedChannel` en vez de UnixStream. Para `ExecStream`, usa
///    `handle_exec_stream_enc`.
///
/// **Limitación v1**: a diferencia del path Unix, mid-stream cancel
/// del cliente sólo se detecta cuando el daemon intenta escribir el
/// próximo evento. Para procesos silenciosos (p. ej. `sleep 30` sin
/// salida), un cliente que cierra TCP no dispara el kill hasta que el
/// proceso emita algo. Se mejora cuando se añada un frame Cancel del
/// cliente o se splittea el FramedChannel en mitades sender/receiver.
async fn handle_enc_client(
    tcp: tokio::net::TcpStream,
    our_keypair: shuma_link::Keypair,
    peers: KnownPeers,
    mgr: Arc<WorkspaceManager>,
    disc: Arc<DiscernPipeline>,
    pool: Option<Arc<card_sidecar::SidecarPool>>,
    daemon_started: std::time::Instant,
) -> anyhow::Result<()> {
    let (mut ch, peer) = shuma_link::server_handshake(tcp, &our_keypair)
        .await
        .map_err(|e| anyhow::anyhow!("handshake: {e}"))?;
    if !peers.contains(&peer) {
        warn!(peer = %peer.to_hex(), "TCP peer no autorizado — rechazando");
        // Cerramos sin enviar nada: no le damos al atacante señal de
        // si la pubkey existió alguna vez (timing-uniform reject).
        return Ok(());
    }
    info!(peer = %peer.to_hex(), "TCP peer autorizado, sirviendo");

    loop {
        let req: Request = match ch.recv_postcard().await {
            Ok(r) => r,
            Err(shuma_link::channel::FrameError::Closed) => return Ok(()),
            Err(e) => return Err(anyhow::anyhow!("recv: {e}")),
        };
        // El uid 0 a `audit_request` no es real — sólo un placeholder.
        // En el log de auditoría aparece como uid=0 para conexiones TCP;
        // la pubkey del peer ya quedó en el log "TCP peer autorizado".
        audit_request(0, &req);

        if let Request::ExecStream { cwd, exec, capture_limit_bytes, stdin_data } = req {
            handle_exec_stream_enc(&mut ch, cwd, exec, capture_limit_bytes, stdin_data).await?;
            continue;
        }

        let resp = dispatch(&mgr, &disc, &pool, daemon_started, req).await;
        if let Err(e) = ch.send_postcard(&resp).await {
            return Err(anyhow::anyhow!("send: {e}"));
        }
    }
}

/// Versión encriptada de [`handle_exec_stream`]. Misma forma: traduce
/// la request al spec, spawnea con `shuma-exec`, puentea sync→async y
/// emite frames de `Response::Exec*` hasta el terminal. La diferencia
/// es que usa el `FramedChannel` en vez de `write_frame` directo, así
/// que cada postcard viaja cifrado y autenticado.
async fn handle_exec_stream_enc<S>(
    ch: &mut FramedChannel<S>,
    cwd: String,
    exec: ProtoExecKind,
    capture_limit_bytes: usize,
    stdin_data: Option<String>,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    let exec_local = match exec {
        ProtoExecKind::Shell { line, program } => shuma_exec::Exec::Shell { line, program },
        ProtoExecKind::Direct { stages } => shuma_exec::Exec::Direct {
            stages: stages
                .into_iter()
                .map(|s| shuma_exec::StageSpec { program: s.program, args: s.args })
                .collect(),
        },
    };
    let spec = shuma_exec::CommandSpec {
        exec: exec_local,
        cwd,
        capture_limit: capture_limit_bytes,
        spill_path: None,
        stdin_data,
    };
    let mut handle = shuma_exec::run(&spec);
    let killer = handle.killer();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<shuma_exec::RunEvent>();
    let _bridge = std::thread::spawn(move || {
        while let Some(ev) = handle.next_event() {
            if tx.send(ev).is_err() {
                return;
            }
        }
    });
    while let Some(ev) = rx.recv().await {
        let terminal = matches!(
            ev,
            shuma_exec::RunEvent::Exited(_) | shuma_exec::RunEvent::Failed(_)
        );
        let resp = exec_event_to_response(ev);
        if let Err(e) = ch.send_postcard(&resp).await {
            // Cliente cerró: matamos al hijo. No drenamos rx — el
            // bridge thread saldrá solo cuando los readers vean EOF.
            killer.kill();
            return Err(anyhow::anyhow!("send: {e}"));
        }
        if terminal {
            break;
        }
    }
    Ok(())
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
        Request::ExecStream { cwd, exec, .. } => {
            let summary = match exec {
                ProtoExecKind::Shell { line, .. } => format!("shell={line:?}"),
                ProtoExecKind::Direct { stages } => stages
                    .iter()
                    .map(|s| s.program.as_str())
                    .collect::<Vec<_>>()
                    .join(" | "),
            };
            ("exec.stream", format!("cwd={cwd} {summary}"))
        }
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
    pool: &Option<Arc<card_sidecar::SidecarPool>>,
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

        // `ExecStream` se atiende inline en `handle_client` con el
        // subprotocolo de streaming; nunca debería llegar aquí. Si lo
        // hace, devolvemos un error explícito en vez de panic.
        Request::ExecStream { .. } => Response::Error {
            message: "ExecStream debe atenderse en handle_client; no por dispatch".into(),
        },
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
    pool: Option<&card_sidecar::SidecarPool>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use shuma_protocol::{ExecKind, ExecStage};

    /// El subprotocolo ExecStream sirve un `echo` end-to-end sobre un par
    /// de UnixStream — sin lanzar el binario del daemon. Comprueba que
    /// los frames llegan en el orden esperado y terminan con `ExecExited`.
    #[tokio::test]
    async fn exec_stream_echoes_a_line() {
        let (mut server, mut client) = tokio::net::UnixStream::pair().unwrap();
        // Server: corre el handler de streaming en tarea aparte.
        let server_task = tokio::spawn(async move {
            handle_exec_stream(
                &mut server,
                ".".into(),
                ProtoExecKind::Direct {
                    stages: vec![ExecStage {
                        program: "echo".into(),
                        args: vec!["hola".into(), "mundo".into()],
                    }],
                },
                0,
                None,
            )
            .await
            .unwrap();
        });
        // Cliente: drena hasta terminal.
        let mut frames: Vec<Response> = Vec::new();
        loop {
            let r: Response = read_frame(&mut client).await.expect("read frame");
            let terminal = r.is_exec_terminal();
            frames.push(r);
            if terminal {
                break;
            }
        }
        server_task.await.unwrap();
        let _ = ExecKind::Direct { stages: vec![] }; // silencia warning si quedara
        // Esperamos al menos: un Stdout("hola mundo") y un ExecExited(0).
        assert!(
            frames.iter().any(|f| matches!(f, Response::ExecStdout(l) if l == "hola mundo")),
            "no llegó la línea de stdout: {frames:?}"
        );
        assert!(matches!(frames.last(), Some(Response::ExecExited(0))));
    }

    /// El daemon mata el proceso si el cliente cierra mitad de stream —
    /// invariante crítica para no dejar zombies (convención SSH/PTY).
    #[tokio::test]
    async fn exec_stream_kills_child_on_client_disconnect() {
        let (mut server, client) = tokio::net::UnixStream::pair().unwrap();
        let server_task = tokio::spawn(async move {
            // `sleep 30` daría tiempo de sobra para detectar zombies; el
            // test debería terminar en <1s gracias al EOF de write_frame.
            let res = handle_exec_stream(
                &mut server,
                ".".into(),
                ProtoExecKind::Direct {
                    stages: vec![ExecStage {
                        program: "sleep".into(),
                        args: vec!["30".into()],
                    }],
                },
                0,
                None,
            )
            .await;
            // Esperamos un error de I/O al intentar escribir tras el close;
            // lo que importa es que la función retornó (no se colgó).
            res
        });
        // Le damos tiempo a arrancar el proceso, luego cerramos.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        drop(client);
        // Sin timeout aquí porque el daemon debe terminar rápido al
        // detectar el EOF al escribir. Si el test cuelga, el invariante
        // está roto.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), server_task)
            .await
            .expect("handler colgado: posible zombie");
    }

    /// `ExecStream::Shell` también funciona (el daemon traduce el variante).
    #[tokio::test]
    async fn exec_stream_supports_shell_mode() {
        let (mut server, mut client) = tokio::net::UnixStream::pair().unwrap();
        let server_task = tokio::spawn(async move {
            handle_exec_stream(
                &mut server,
                ".".into(),
                ProtoExecKind::Shell {
                    line: "echo $((2 + 3))".into(),
                    program: "sh".into(),
                },
                0,
                None,
            )
            .await
            .unwrap();
        });
        let mut got = String::new();
        loop {
            let r: Response = read_frame(&mut client).await.unwrap();
            if let Response::ExecStdout(l) = &r {
                got = l.clone();
            }
            if r.is_exec_terminal() {
                break;
            }
        }
        server_task.await.unwrap();
        assert_eq!(got, "5");
    }
}
