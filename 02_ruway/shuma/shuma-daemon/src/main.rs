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

mod pty_sessions;
use pty_sessions::{PtyRegistry, SessionEvent};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let sock = default_socket_path();
    let pid_path = pid_path_for(&sock);

    // 1) Lock exclusivo no-bloqueante sobre el `.pid` — singleton garantizado
    //    al nivel del kernel. Si falla, OTRO shuma-daemon vivo lo tiene y
    //    abortamos con un mensaje claro (incluye el PID dueño). El handle
    //    se retiene hasta el final de `main`: cuando se drop, el OS libera
    //    el lock automáticamente (incluso si crasheamos sin Drop ordenado).
    let _lockfile = acquire_lockfile(&pid_path).with_context(|| {
        format!(
            "no se pudo adquirir el lockfile {} — ¿hay otro shuma-daemon corriendo?",
            pid_path.display()
        )
    })?;

    // 2) Defensa adicional contra socket vivo (el lockfile protege contra
    //    dos instancias del mismo usuario; este chequeo cubre el caso de
    //    socket dejado por otro usuario o por un proceso que escapó del
    //    lock por compartir directorio).
    if socket_in_use(&sock) {
        anyhow::bail!(
            "el socket {} ya está atendido por otro proceso — abortando para no pisarlo",
            sock.display()
        );
    }

    if sock.exists() {
        // Socket stale (post-crash): el lockfile ya garantizó que no hay
        // otro daemon vivo, así que es seguro barrerlo.
        let _ = std::fs::remove_file(&sock);
    }
    if let Some(parent) = sock.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let listener = UnixListener::bind(&sock).with_context(|| format!("bind {}", sock.display()))?;
    info!(socket = %sock.display(), pid_lock = %pid_path.display(), "shuma-daemon listening");
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

    // Registro de sesiones PTY persistentes (tmux-like). Compartido por
    // los dos listeners; vive lo que viva el daemon.
    let pty_registry = Arc::new(PtyRegistry::default());

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
        let pty_tcp = pty_registry.clone();
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
                        let pty = pty_tcp.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_enc_client(
                                tcp,
                                our_kp,
                                peers,
                                mgr,
                                disc,
                                pool,
                                pty,
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
                let pty = pty_registry.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, mgr, disc, pool, pty, daemon_started).await {
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
    pty: Arc<PtyRegistry>,
    daemon_started: std::time::Instant,
) -> anyhow::Result<()> {
    // Audit: peer uid lo leemos una vez aquí (no cambia durante la conexión).
    let peer_id = match peer_uid(&stream) {
        Ok(u) => format!("uid:{u}"),
        Err(_) => "uid:unknown".to_string(),
    };
    loop {
        let req: Request = match read_frame(&mut stream).await {
            Ok(r) => r,
            Err(shuma_protocol::ProtocolError::Closed) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        audit_request(&peer_id, &req);

        // El subprotocolo `ExecStream` produce N frames sobre la misma
        // conexión hasta un terminal. Lo manejamos inline en vez de pasar
        // por `dispatch` (que es request/response 1:1).
        if let Request::ExecStream { cwd, exec, capture_limit_bytes, stdin_data, capture_stages } = req {
            handle_exec_stream(&mut stream, cwd, exec, capture_limit_bytes, stdin_data, capture_stages).await?;
            continue;
        }
        // PTY remoto: la conexión pasa a modo full-duplex y se consume
        // hasta el exit (cierra al terminar, como el path cifrado).
        if let Request::ExecPty { cwd, program, args, rows, cols } = req {
            return handle_pty_stream(stream, cwd, program, args, rows, cols).await;
        }
        // Adjuntarse a una sesión persistente: full-duplex hasta que el
        // cliente cierra (DETACH) o el proceso de la sesión muere.
        if let Request::PtyAttach { session, rows, cols } = req {
            return handle_pty_attach(stream, &pty, session, rows, cols).await;
        }

        let resp = dispatch(&mgr, &disc, &pool, &pty, daemon_started, req).await;
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
    capture_stages: bool,
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
        capture_stages,
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
        // Salida de una etapa intermedia del pipe (tee). Sólo aparece si
        // el cliente pidió `capture_stages: true`; la reemitimos como su
        // propio frame para que el cliente la pinte en el desplegable de
        // la etapa correspondiente, no mezclada con el stdout final.
        shuma_exec::RunEvent::StageStdout { stage, line } => {
            Response::ExecStageStdout { stage, line }
        }
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

/// Traductor del lado PTY: lo que produce `Exec::Pty` son `Bytes` crudos
/// (la salida del terminal); el resto de variantes no debería aparecer.
fn pty_event_to_response(ev: shuma_exec::RunEvent) -> Response {
    match ev {
        shuma_exec::RunEvent::Bytes(b) => Response::ExecBytes(b),
        shuma_exec::RunEvent::Exited(c) => Response::ExecExited(c),
        shuma_exec::RunEvent::Failed(m) => Response::ExecFailed(m),
        // Un PTY captura a su propia pantalla (vt100), no a buffers de
        // línea; estas variantes no deberían darse. Si pasan, las
        // reemitimos como bytes para no perderlas.
        shuma_exec::RunEvent::Stdout(l) | shuma_exec::RunEvent::Stderr(l) => {
            Response::ExecBytes(l.into_bytes())
        }
        shuma_exec::RunEvent::StageStdout { line, .. } => Response::ExecBytes(line.into_bytes()),
        shuma_exec::RunEvent::Truncated | shuma_exec::RunEvent::Spilled(_) => {
            Response::ExecBytes(Vec::new())
        }
    }
}

/// Subprotocolo `ExecPty` (texto plano): spawnea un PTY y multiplexa la
/// conexión en full-duplex. Una **tarea lectora** dedicada decodifica los
/// frames del cliente (`PtyInput`/`PtyResize`) y maneja el PTY, mientras
/// el loop principal escribe la salida del terminal (`ExecBytes`). Separar
/// lectura y escritura en mitades owned evita cancelar un `read_frame` a
/// mitad de frame (cancel-safety) sin malabares de borrow.
async fn handle_pty_stream(
    stream: UnixStream,
    cwd: String,
    program: String,
    args: Vec<String>,
    rows: u16,
    cols: u16,
) -> anyhow::Result<()> {
    let spec = pty_spec(cwd, program, args, rows, cols);
    let mut handle = shuma_exec::run(&spec);
    let killer = handle.killer();
    let pty = handle.pty_control();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<shuma_exec::RunEvent>();
    let _bridge = std::thread::spawn(move || {
        while let Some(ev) = handle.next_event() {
            if tx.send(ev).is_err() {
                return;
            }
        }
    });

    let (mut rd, mut wr) = tokio::io::split(stream);
    // Tarea lectora: drena frames del cliente y maneja el PTY directo.
    let reader = tokio::spawn(async move {
        loop {
            match read_frame::<Request, _>(&mut rd).await {
                Ok(Request::PtyInput { bytes }) => {
                    pty.write_input(bytes);
                }
                Ok(Request::PtyResize { rows, cols }) => {
                    pty.resize(rows, cols);
                }
                Ok(_) => {} // frame fuera del protocolo PTY: ignorar
                Err(_) => {
                    // EOF/error = cliente cerró → matar el PTY (SSH).
                    killer.kill();
                    break;
                }
            }
        }
    });

    // Loop de escritura: salida del terminal hasta el terminal.
    while let Some(ev) = rx.recv().await {
        let terminal = matches!(
            ev,
            shuma_exec::RunEvent::Exited(_) | shuma_exec::RunEvent::Failed(_)
        );
        let resp = pty_event_to_response(ev);
        if write_frame(&mut wr, &resp).await.is_err() {
            break;
        }
        if terminal {
            break;
        }
    }
    reader.abort();
    Ok(())
}

/// Versión cifrada de [`handle_pty_stream`]. Consume el `FramedChannel`
/// (el `split` es owned) y usa sus mitades `FramedReader`/`FramedWriter`.
async fn handle_pty_stream_enc<S>(
    ch: FramedChannel<S>,
    cwd: String,
    program: String,
    args: Vec<String>,
    rows: u16,
    cols: u16,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let spec = pty_spec(cwd, program, args, rows, cols);
    let mut handle = shuma_exec::run(&spec);
    let killer = handle.killer();
    let pty = handle.pty_control();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<shuma_exec::RunEvent>();
    let _bridge = std::thread::spawn(move || {
        while let Some(ev) = handle.next_event() {
            if tx.send(ev).is_err() {
                return;
            }
        }
    });

    let (mut rd, mut wr) = ch.split();
    let reader = tokio::spawn(async move {
        loop {
            match rd.recv_postcard::<Request>().await {
                Ok(Request::PtyInput { bytes }) => {
                    pty.write_input(bytes);
                }
                Ok(Request::PtyResize { rows, cols }) => {
                    pty.resize(rows, cols);
                }
                Ok(_) => {}
                Err(_) => {
                    killer.kill();
                    break;
                }
            }
        }
    });

    while let Some(ev) = rx.recv().await {
        let terminal = matches!(
            ev,
            shuma_exec::RunEvent::Exited(_) | shuma_exec::RunEvent::Failed(_)
        );
        let resp = pty_event_to_response(ev);
        if wr.send_postcard(&resp).await.is_err() {
            break;
        }
        if terminal {
            break;
        }
    }
    reader.abort();
    Ok(())
}

/// `CommandSpec` para un PTY remoto — común a los dos handlers.
fn pty_spec(
    cwd: String,
    program: String,
    args: Vec<String>,
    rows: u16,
    cols: u16,
) -> shuma_exec::CommandSpec {
    shuma_exec::CommandSpec {
        exec: shuma_exec::Exec::Pty { program, args, cols, rows },
        cwd,
        capture_limit: 0,
        spill_path: None,
        stdin_data: None,
        capture_stages: false,
    }
}

/// Adjunta una conexión Unix a una sesión PTY persistente: manda el
/// scrollback, luego la salida en vivo, y reenvía teclas/resizes al PTY.
/// Cerrar la conexión = **DETACH** (no mata la sesión); la sesión sólo
/// muere al terminar su proceso o por `PtyKill`.
async fn handle_pty_attach(
    stream: UnixStream,
    pty: &Arc<PtyRegistry>,
    session: ulid::Ulid,
    rows: u16,
    cols: u16,
) -> anyhow::Result<()> {
    let (mut rd, mut wr) = tokio::io::split(stream);
    let Some(sess) = pty.get(session) else {
        let _ = write_frame(
            &mut wr,
            &Response::ExecFailed(format!("sesión {session} no existe")),
        )
        .await;
        return Ok(());
    };
    sess.resize(rows, cols);
    let att = sess.attach();

    // Tarea lectora: teclas/resizes del cliente → PTY. EOF/error = el
    // cliente cerró → DETACH (no matamos la sesión, sólo salimos).
    let sess_in = Arc::clone(&sess);
    let reader = tokio::spawn(async move {
        loop {
            match read_frame::<Request, _>(&mut rd).await {
                Ok(Request::PtyInput { bytes }) => sess_in.write_input(bytes),
                Ok(Request::PtyResize { rows, cols }) => sess_in.resize(rows, cols),
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    // Scrollback inicial para repintar la pantalla.
    if !att.scrollback.is_empty()
        && write_frame(&mut wr, &Response::ExecBytes(att.scrollback))
            .await
            .is_err()
    {
        reader.abort();
        return Ok(());
    }
    // Sesión ya muerta al adjuntarse: el `Exited` ya se broadcasteó antes
    // de nuestra suscripción, así que lo sintetizamos y cerramos.
    if let Some(code) = att.exited {
        let _ = write_frame(&mut wr, &Response::ExecExited(code)).await;
        reader.abort();
        return Ok(());
    }
    let mut rx = att.rx;
    loop {
        match rx.recv().await {
            Ok(SessionEvent::Bytes(b)) => {
                if write_frame(&mut wr, &Response::ExecBytes(b.as_ref().clone()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(SessionEvent::Exited(c)) => {
                let _ = write_frame(&mut wr, &Response::ExecExited(c)).await;
                break;
            }
            // El cliente quedó atrás: repintamos el ring entero en vez de
            // arrastrar bytes perdidos (corromperían el vt100).
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                if write_frame(&mut wr, &Response::ExecBytes(sess.scrollback()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
    reader.abort();
    Ok(())
}

/// Versión cifrada de [`handle_pty_attach`] sobre un `FramedChannel`.
async fn handle_pty_attach_enc<S>(
    ch: FramedChannel<S>,
    pty: &Arc<PtyRegistry>,
    session: ulid::Ulid,
    rows: u16,
    cols: u16,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut rd, mut wr) = ch.split();
    let Some(sess) = pty.get(session) else {
        let _ = wr
            .send_postcard(&Response::ExecFailed(format!("sesión {session} no existe")))
            .await;
        return Ok(());
    };
    sess.resize(rows, cols);
    let att = sess.attach();

    let sess_in = Arc::clone(&sess);
    let reader = tokio::spawn(async move {
        loop {
            match rd.recv_postcard::<Request>().await {
                Ok(Request::PtyInput { bytes }) => sess_in.write_input(bytes),
                Ok(Request::PtyResize { rows, cols }) => sess_in.resize(rows, cols),
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    if !att.scrollback.is_empty()
        && wr
            .send_postcard(&Response::ExecBytes(att.scrollback))
            .await
            .is_err()
    {
        reader.abort();
        return Ok(());
    }
    if let Some(code) = att.exited {
        let _ = wr.send_postcard(&Response::ExecExited(code)).await;
        reader.abort();
        return Ok(());
    }
    let mut rx = att.rx;
    loop {
        match rx.recv().await {
            Ok(SessionEvent::Bytes(b)) => {
                if wr
                    .send_postcard(&Response::ExecBytes(b.as_ref().clone()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(SessionEvent::Exited(c)) => {
                let _ = wr.send_postcard(&Response::ExecExited(c)).await;
                break;
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                if wr
                    .send_postcard(&Response::ExecBytes(sess.scrollback()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
    reader.abort();
    Ok(())
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
#[allow(clippy::too_many_arguments)]
async fn handle_enc_client(
    tcp: tokio::net::TcpStream,
    our_keypair: shuma_link::Keypair,
    peers: KnownPeers,
    mgr: Arc<WorkspaceManager>,
    disc: Arc<DiscernPipeline>,
    pool: Option<Arc<card_sidecar::SidecarPool>>,
    pty: Arc<PtyRegistry>,
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
    // Identidad del peer en el audit log: primeros 16 hex de su pubkey
    // X25519 — suficientes para correlacionar entradas sin volcar la
    // clave entera en cada línea.
    let peer_hex = peer.to_hex();
    let peer_id = format!("pubkey:{}", &peer_hex[..peer_hex.len().min(16)]);

    loop {
        let req: Request = match ch.recv_postcard().await {
            Ok(r) => r,
            Err(shuma_link::channel::FrameError::Closed) => return Ok(()),
            Err(e) => return Err(anyhow::anyhow!("recv: {e}")),
        };
        audit_request(&peer_id, &req);

        if let Request::ExecStream { cwd, exec, capture_limit_bytes, stdin_data, capture_stages } = req {
            handle_exec_stream_enc(&mut ch, cwd, exec, capture_limit_bytes, stdin_data, capture_stages).await?;
            continue;
        }
        // PTY remoto cifrado: consume el canal (el split es owned) y cierra
        // la conexión al terminar — el cliente abre una conexión dedicada
        // por sesión PTY, igual que con los runs no-PTY.
        if let Request::ExecPty { cwd, program, args, rows, cols } = req {
            return handle_pty_stream_enc(ch, cwd, program, args, rows, cols).await;
        }
        // Adjuntarse a una sesión persistente sobre el canal cifrado.
        if let Request::PtyAttach { session, rows, cols } = req {
            return handle_pty_attach_enc(ch, &pty, session, rows, cols).await;
        }

        let resp = dispatch(&mgr, &disc, &pool, &pty, daemon_started, req).await;
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
    capture_stages: bool,
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
        capture_stages,
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

/// Loguea cada mutación con target="audit" y el peer. Reads (ping,
/// list, stats) se omiten para no inundar el log. `peer` es opaco:
/// para Unix sockets viene como `"uid:1000"` (de `SO_PEERCRED`); para
/// TCP autenticado viene como `"pubkey:abcdef…"` (los 16 primeros hex
/// de la X25519 pública del peer).
fn audit_request(peer: &str, req: &Request) {
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
        Request::ExecPty { cwd, program, args, .. } => (
            "exec.pty",
            format!("cwd={cwd} {program} {}", args.join(" ")),
        ),
        Request::PtySpawn { cwd, program, args, label, .. } => (
            "pty.spawn",
            format!("cwd={cwd} label={label:?} {program} {}", args.join(" ")),
        ),
        Request::PtyAttach { session, .. } => ("pty.attach", format!("session={session}")),
        Request::PtyKill { session } => ("pty.kill", format!("session={session}")),
        // Reads / alta frecuencia (no audit). Las teclas y resizes de un
        // PTY no se auditan línea a línea — la apertura (`exec.pty` /
        // `pty.spawn`) ya quedó registrada.
        Request::PtyInput { .. }
        | Request::PtyResize { .. }
        | Request::PtyList
        | Request::Ping
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
    info!(target: "audit", peer, action, detail = %detail, "audit");
    // Append a file. Failure no es fatal — sólo se pierde la entry.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let line = format!("ts={ts} peer={peer} action={action} {detail}");
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
    pty: &Arc<PtyRegistry>,
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

        Request::WorkspaceCreate { spec } => {
            let label_clone = spec.label.clone();
            match mgr.create(spec).await {
                Ok((id, warnings)) => {
                    if let Some(p) = pool.as_deref() {
                        p.spawn(build_workspace_card(&label_clone, id));
                    }
                    Response::WorkspaceCreated { id, warnings }
                }
                Err(e) => Response::Error { message: format!("{e}") },
            }
        }

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

        // Sesiones PTY persistentes. `PtySpawn`/`PtyList`/`PtyKill` son
        // request/response 1:1 y se atienden aquí; `PtyAttach` es
        // full-duplex y se intercepta inline antes de `dispatch`.
        Request::PtySpawn { cwd, program, args, rows, cols, label } => {
            let session = pty.spawn(cwd, program, args, rows, cols, label);
            Response::PtySpawned { session }
        }
        Request::PtyList => Response::PtyList { sessions: pty.list() },
        Request::PtyKill { session } => Response::PtyKilled {
            session,
            existed: pty.kill(session),
        },

        // `ExecStream`/`ExecPty`/`PtyAttach` se atienden inline en los
        // handlers de conexión con sus subprotocolos full-duplex; los
        // frames `PtyInput`/`PtyResize` sólo viven dentro de uno ya en
        // curso. Nunca deberían llegar a `dispatch` (request/response 1:1).
        Request::ExecStream { .. }
        | Request::ExecPty { .. }
        | Request::PtyAttach { .. }
        | Request::PtyInput { .. }
        | Request::PtyResize { .. } => Response::Error {
            message: "frame de streaming/PTY fuera de su subprotocolo; no por dispatch".into(),
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

/// Card de un Workspace recién creado. Se publica al pool para que el
/// broker-explorer (y consumidores futuros) puedan listar los
/// workspaces vivos sin pasar por el daemon. La card es `Ente`
/// (entidad viva), `Lifecycle::Daemon` (vive lo que el workspace), y
/// expone un flow `commands` que un consumidor podría suscribir.
fn build_workspace_card(label: &str, id: shuma_card::WorkspaceId) -> Card {
    let card_label = format!("shuma.workspace.{}.{}", short_workspace_id(&id), label);
    let mut card = Card::new(card_label);
    card.kind = CardKind::Ente;
    card.lifecycle = Lifecycle::Daemon;
    card.payload = Payload::Virtual;
    card.supervision = Supervision::Delegate;
    card.flow = Flows {
        input: Vec::new(),
        output: vec![Flow {
            name: "commands".into(),
            ty: TypeRef::Wit {
                package: "shuma:admin".into(),
                interface: None,
                name: "command-list".into(),
            },
            pin_to: None,
        }],
    };
    card
}

fn short_workspace_id(id: &shuma_card::WorkspaceId) -> String {
    let s = id.to_string();
    s[s.len().saturating_sub(6)..].to_string()
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

/// Path del lockfile asociado al socket admin: mismo dir, extensión `.pid`.
fn pid_path_for(sock: &std::path::Path) -> std::path::PathBuf {
    sock.with_extension("pid")
}

/// ¿Hay un peer atendiendo el socket admin? Distingue stale (post-crash)
/// de vivo (otro daemon). Mismo patrón que arje-zero y sandokan-local.
fn socket_in_use(path: &std::path::Path) -> bool {
    if !path.exists() {
        return false;
    }
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

/// Adquiere un lock exclusivo no-bloqueante sobre `pid_path`, escribe el
/// PID actual y devuelve el `File` que sostiene el lock. Mientras el
/// `File` viva, el kernel garantiza que ningún otro proceso adquiera el
/// mismo lock (advisory, pero todos los daemones cooperan al llamar esto
/// antes de bindear). Cuando el `File` se drop —Drop ordenado o crash—,
/// el OS libera el lock; el `.pid` queda en disco pero ya no protege.
fn acquire_lockfile(pid_path: &std::path::Path) -> anyhow::Result<std::fs::File> {
    use std::io::{Seek, SeekFrom, Write};
    use std::os::fd::AsRawFd;

    if let Some(parent) = pid_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // No truncamos al abrir: si flock falla, queremos preservar el PID
    // viejo para que el mensaje de error sea informativo.
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(pid_path)
        .with_context(|| format!("abrir {}", pid_path.display()))?;

    let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if r != 0 {
        let err = std::io::Error::last_os_error();
        let other_pid = std::fs::read_to_string(pid_path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "?".to_string());
        anyhow::bail!(
            "lockfile {} ya tomado por PID {} ({err})",
            pid_path.display(),
            other_pid,
        );
    }

    // Tenemos el lock: actualizamos el contenido al PID actual.
    f.set_len(0)?;
    f.seek(SeekFrom::Start(0))?;
    writeln!(f, "{}", std::process::id())?;
    f.flush()?;
    Ok(f)
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
                false,
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
                false,
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
                false,
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

    /// Con `capture_stages: true`, un pipe `Direct` de dos etapas debe
    /// emitir el tee de la etapa intermedia como frames
    /// `ExecStageStdout { stage: 0, .. }` además del stdout final. Es la
    /// invariante del "live tee" sobre el transporte del daemon.
    #[tokio::test]
    async fn exec_stream_tees_intermediate_stage() {
        let (mut server, mut client) = tokio::net::UnixStream::pair().unwrap();
        let server_task = tokio::spawn(async move {
            handle_exec_stream(
                &mut server,
                ".".into(),
                // `printf "a\nb\n" | cat` — la etapa 0 (printf) es la
                // intermedia que se intercepta; la 1 (cat) da el final.
                ProtoExecKind::Direct {
                    stages: vec![
                        ExecStage {
                            program: "printf".into(),
                            args: vec!["a\\nb\\n".into()],
                        },
                        ExecStage { program: "cat".into(), args: vec![] },
                    ],
                },
                0,
                None,
                true,
            )
            .await
            .unwrap();
        });
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
        // Debe haber al menos un frame de tee de la etapa 0.
        assert!(
            frames
                .iter()
                .any(|f| matches!(f, Response::ExecStageStdout { stage: 0, line } if line == "a")),
            "no llegó el tee de la etapa intermedia: {frames:?}"
        );
        // Y el stdout final ("b" o "a"/"b") debe seguir llegando aparte.
        assert!(
            frames.iter().any(|f| matches!(f, Response::ExecStdout(_))),
            "no llegó el stdout final: {frames:?}"
        );
        assert!(matches!(frames.last(), Some(Response::ExecExited(0))));
    }

    // -----------------------------------------------------------------
    //  Singleton del daemon: socket_in_use + flock(LOCK_EX | LOCK_NB)
    // -----------------------------------------------------------------

    #[test]
    fn pid_path_es_sock_con_extension_pid() {
        let p = pid_path_for(std::path::Path::new("/run/foo.sock"));
        assert_eq!(p, std::path::PathBuf::from("/run/foo.pid"));
    }

    #[test]
    fn socket_in_use_false_para_path_inexistente() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join("ausente.sock");
        assert!(!socket_in_use(&p));
    }

    #[test]
    fn lockfile_se_adquiere_y_escribe_el_pid_actual() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join("shuma.pid");
        let _guard = acquire_lockfile(&p).expect("primer lock");
        let leido = std::fs::read_to_string(&p).expect("read pid");
        assert_eq!(leido.trim(), std::process::id().to_string());
    }

    #[test]
    fn lockfile_segundo_lock_en_el_mismo_archivo_falla() {
        // Dos handles distintos al MISMO path: el segundo flock debe
        // fallar con EWOULDBLOCK. Equivale al escenario "dos daemones
        // arrancan a la vez".
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join("compit.pid");
        let _primer = acquire_lockfile(&p).expect("primer lock");
        let err = acquire_lockfile(&p)
            .expect_err("el segundo debe ser rechazado por el kernel");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("ya tomado") || msg.contains("PID"),
            "mensaje no enuncia conflicto: {msg}"
        );
    }

    #[test]
    fn lockfile_drop_del_primer_handle_libera_el_lock() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join("relevo.pid");
        // Sacamos el lock + lo droppeamos.
        let primer = acquire_lockfile(&p).expect("primer lock");
        drop(primer);
        // El segundo debe tomar el lock sin error.
        let _segundo =
            acquire_lockfile(&p).expect("tras drop del primero, el segundo debe poder lockear");
    }
}
