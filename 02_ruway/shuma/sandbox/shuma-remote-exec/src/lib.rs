//! `shuma-remote-exec` — cliente **sync** del subprotocolo
//! [`ExecStream`](shuma_protocol::Request::ExecStream) del daemon.
//!
//! El shell GPUI no es async; ejecuta comandos a través de
//! `shuma-exec` (un crate puramente sync con threads + mpsc) y drena
//! los eventos con `try_events()`. Este crate provee la misma forma —
//! [`RemoteRunHandle`] paralelo a [`shuma_exec::RunHandle`] — pero los
//! eventos vienen del **daemon** por Unix socket, no del proceso local.
//!
//! Eso permite que el shell sea cliente delgado contra `shuma-daemon`:
//! una vez que el transporte abstrae la conexión, sustituir el Unix
//! socket por una conexión autenticada (Bloque 7) o un túnel SSH es un
//! cambio de implementación, no de API.
//!
//! Arquitectura interna:
//!
//! ```text
//!   shell (sync) ──┐
//!                  ├─ try_events / kill / is_finished (idéntico a shuma-exec)
//!  RemoteRunHandle ┤
//!                  └─ background thread: tokio runtime ─ UnixStream ─ daemon
//! ```
//!
//! Un único hilo dedicado abre su propio runtime de tokio y conecta al
//! socket; lee frames y los reemite como `RunEvent`s por un canal
//! mpsc estándar. `kill()` cierra el stream — el daemon detecta el
//! EOF y mata al proceso hijo (convención SSH/PTY).

#![forbid(unsafe_code)]

pub use shuma_exec::RunEvent;
use shuma_exec::{CommandSpec, Exec};
use shuma_protocol::{
    read_frame, write_frame, ExecKind, ExecStage, Request, Response,
};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use tokio::sync::Notify;

/// Asa de un comando que se ejecuta en el daemon. API a propósito
/// idéntica (en spirit) a [`shuma_exec::RunHandle`] para que el shell
/// pueda usar ambos detrás del mismo trait.
pub struct RemoteRunHandle {
    rx: Receiver<RunEvent>,
    finished: bool,
    /// Señalización al hilo de fondo para cancelar (cerrar el stream).
    /// El daemon detecta el EOF y mata al proceso.
    cancel: Arc<Notify>,
}

impl RemoteRunHandle {
    /// Mata el proceso remoto cerrando el stream — el daemon lo detecta
    /// y dispara SIGKILL en el proceso hijo.
    pub fn kill(&self) {
        self.cancel.notify_waiters();
    }

    /// Drena eventos disponibles ahora, sin bloquear.
    pub fn try_events(&mut self) -> Vec<RunEvent> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(ev) => {
                    if matches!(ev, RunEvent::Exited(_) | RunEvent::Failed(_)) {
                        self.finished = true;
                    }
                    out.push(ev);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.finished = true;
                    break;
                }
            }
        }
        out
    }

    /// Bloquea hasta el próximo evento. `None` cuando terminó.
    pub fn next_event(&mut self) -> Option<RunEvent> {
        if self.finished {
            return None;
        }
        match self.rx.recv() {
            Ok(ev) => {
                if matches!(ev, RunEvent::Exited(_) | RunEvent::Failed(_)) {
                    self.finished = true;
                }
                Some(ev)
            }
            Err(_) => {
                self.finished = true;
                None
            }
        }
    }

    /// `true` si el evento terminal ya pasó.
    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

/// Errores que el shell puede ver al pedir un run remoto. La política
/// es no-cae-el-shell: si el daemon no contesta, el caller traduce el
/// error a un `RunEvent::Failed` y continúa.
#[derive(Debug, thiserror::Error)]
pub enum RemoteExecError {
    #[error("conexión Unix a {0}: {1}")]
    Connect(PathBuf, std::io::Error),
    #[error("conexión TCP a {0}: {1}")]
    ConnectTcp(String, std::io::Error),
}

/// Lanza `spec` contra el daemon en `socket` y devuelve un asa cuyos
/// eventos llegan en streaming. La función vuelve de inmediato — el
/// trabajo de I/O se hace en un hilo aparte con su propio runtime.
pub fn run(spec: &CommandSpec, socket: &std::path::Path) -> Result<RemoteRunHandle, RemoteExecError> {
    let (tx, rx) = std::sync::mpsc::channel::<RunEvent>();
    let cancel = Arc::new(Notify::new());
    let cancel_thread = cancel.clone();
    let socket_owned = socket.to_path_buf();

    // Traducción a tipos del protocolo (los del crate `shuma-protocol`
    // son los Serialize; los de `shuma-exec` son los locales).
    let exec_proto = match &spec.exec {
        Exec::Shell { line, program } => ExecKind::Shell {
            line: line.clone(),
            program: program.clone(),
        },
        Exec::Direct { stages } => ExecKind::Direct {
            stages: stages
                .iter()
                .map(|s| ExecStage { program: s.program.clone(), args: s.args.clone() })
                .collect(),
        },
    };
    let req = Request::ExecStream {
        cwd: spec.cwd.clone(),
        exec: exec_proto,
        capture_limit_bytes: spec.capture_limit,
        stdin_data: spec.stdin_data.clone(),
    };

    // Conexión sincronicamente: si falla, devolvemos antes de spawnear
    // el hilo. Así el caller decide qué hacer.
    let std_stream = std::os::unix::net::UnixStream::connect(&socket_owned)
        .map_err(|e| RemoteExecError::Connect(socket_owned.clone(), e))?;
    std_stream
        .set_nonblocking(true)
        .map_err(|e| RemoteExecError::Connect(socket_owned.clone(), e))?;

    std::thread::spawn(move || {
        // Cada hilo de un comando trae su propio runtime current-thread —
        // bajo en overhead, y el shell puede tener varios en vuelo sin
        // contender un runtime global.
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(RunEvent::Failed(format!("runtime: {e}")));
                return;
            }
        };
        rt.block_on(async move {
            let mut stream = match tokio::net::UnixStream::from_std(std_stream) {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(RunEvent::Failed(format!("from_std: {e}")));
                    return;
                }
            };
            if let Err(e) = write_frame(&mut stream, &req).await {
                let _ = tx.send(RunEvent::Failed(format!("write request: {e}")));
                return;
            }
            // Loop: leer frames del daemon, traducirlos a RunEvent y
            // reemitirlos. Si el cliente cancela, cerramos el stream y
            // el daemon mata al hijo.
            loop {
                tokio::select! {
                    biased;
                    _ = cancel_thread.notified() => {
                        // Cerrar — el daemon detectará EOF.
                        drop(stream);
                        return;
                    }
                    res = read_frame::<Response>(&mut stream) => {
                        let resp = match res {
                            Ok(r) => r,
                            Err(e) => {
                                let _ = tx.send(RunEvent::Failed(format!("read frame: {e}")));
                                return;
                            }
                        };
                        let terminal = resp.is_exec_terminal();
                        if let Some(ev) = response_to_event(resp) {
                            if tx.send(ev).is_err() {
                                // El consumidor desapareció: cerrar para
                                // que el daemon mate al hijo.
                                return;
                            }
                        }
                        if terminal {
                            return;
                        }
                    }
                }
            }
        });
    });

    Ok(RemoteRunHandle { rx, finished: false, cancel })
}

fn response_to_event(r: Response) -> Option<RunEvent> {
    match r {
        Response::ExecStarted { .. } => None, // metadato, no es un RunEvent
        Response::ExecStdout(l) => Some(RunEvent::Stdout(l)),
        Response::ExecStderr(l) => Some(RunEvent::Stderr(l)),
        Response::ExecTruncated => Some(RunEvent::Truncated),
        Response::ExecSpilled(p) => Some(RunEvent::Spilled(p)),
        Response::ExecExited(c) => Some(RunEvent::Exited(c)),
        Response::ExecFailed(m) => Some(RunEvent::Failed(m)),
        other => Some(RunEvent::Failed(format!(
            "frame inesperado en stream: {other:?}"
        ))),
    }
}

/// Convenience: usa la ruta canónica del socket que defina
/// [`shuma_protocol::default_socket_path`].
pub fn run_default(spec: &CommandSpec) -> Result<RemoteRunHandle, RemoteExecError> {
    run(spec, &shuma_protocol::default_socket_path())
}

/// Variante autenticada y cifrada vía Noise XK sobre TCP — espejo de
/// [`run`] para hablar con un daemon **remoto**. El cliente conoce de
/// antemano la pubkey del servidor (`server_pub`, igual que
/// `known_hosts` en SSH); el server valida nuestra pubkey contra su
/// propio allowlist.
///
/// El `RemoteRunHandle` que devuelve tiene la misma forma que el del
/// Unix path — el shell consume `try_events / kill / is_finished`
/// igual en los dos casos.
pub fn run_tcp(
    spec: &CommandSpec,
    addr: &str,
    our_keypair: shuma_link::Keypair,
    server_pub: shuma_link::PublicKey,
) -> Result<RemoteRunHandle, RemoteExecError> {
    let (tx, rx) = std::sync::mpsc::channel::<RunEvent>();
    let cancel = Arc::new(Notify::new());
    let cancel_thread = cancel.clone();
    let addr_owned = addr.to_string();

    // Mismo proto Request que el path Unix.
    let exec_proto = match &spec.exec {
        Exec::Shell { line, program } => ExecKind::Shell {
            line: line.clone(),
            program: program.clone(),
        },
        Exec::Direct { stages } => ExecKind::Direct {
            stages: stages
                .iter()
                .map(|s| ExecStage { program: s.program.clone(), args: s.args.clone() })
                .collect(),
        },
    };
    let req = Request::ExecStream {
        cwd: spec.cwd.clone(),
        exec: exec_proto,
        capture_limit_bytes: spec.capture_limit,
        stdin_data: spec.stdin_data.clone(),
    };

    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(RunEvent::Failed(format!("runtime: {e}")));
                return;
            }
        };
        rt.block_on(async move {
            // 1) Conexión TCP. El error de conexión llega como Failed
            // (en vez de Err(RemoteExecError::ConnectTcp)) porque el
            // caller ya recibió el RemoteRunHandle: la forma de
            // notificarle ahora es vía el canal de eventos.
            let tcp = match tokio::net::TcpStream::connect(&addr_owned).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(RunEvent::Failed(format!("connect {addr_owned}: {e}")));
                    return;
                }
            };
            // 2) Handshake Noise XK. Si la pubkey del server no
            // coincide con `server_pub`, falla aquí (protección MITM).
            let mut ch = match shuma_link::client_handshake(tcp, &our_keypair, server_pub).await {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(RunEvent::Failed(format!("handshake: {e}")));
                    return;
                }
            };
            // 3) Misma forma que el Unix path: enviar la request y
            // drenar Response frames hasta el terminal o cancel.
            if let Err(e) = ch.send_postcard(&req).await {
                let _ = tx.send(RunEvent::Failed(format!("send request: {e}")));
                return;
            }
            loop {
                tokio::select! {
                    biased;
                    _ = cancel_thread.notified() => {
                        drop(ch);
                        return;
                    }
                    res = ch.recv_postcard::<Response>() => {
                        let resp = match res {
                            Ok(r) => r,
                            Err(e) => {
                                let _ = tx.send(RunEvent::Failed(format!("read frame: {e}")));
                                return;
                            }
                        };
                        let terminal = resp.is_exec_terminal();
                        if let Some(ev) = response_to_event(resp) {
                            if tx.send(ev).is_err() {
                                return;
                            }
                        }
                        if terminal {
                            return;
                        }
                    }
                }
            }
        });
    });

    Ok(RemoteRunHandle { rx, finished: false, cancel })
}

// Re-exports útiles para el shell.
pub use shuma_exec::{CommandSpec as Spec, Exec as ExecLocal, StageSpec as Stage};

#[cfg(test)]
mod tests {
    use super::*;
    use shuma_exec::StageSpec;
    use shuma_protocol::ExecKind as ProtoExecKind;
    use std::time::Duration;

    /// Mini servidor que sólo entiende ExecStream — simula al daemon
    /// pero sin sus dependencias pesadas (cgroups, etc.). Usa el mismo
    /// puente sync→async que el daemon real.
    async fn serve_exec_stream(mut stream: tokio::net::UnixStream) {
        let req: Request = read_frame(&mut stream).await.expect("read request");
        let Request::ExecStream { cwd, exec, capture_limit_bytes, stdin_data } = req else {
            panic!("esperaba ExecStream");
        };
        let exec_local = match exec {
            ProtoExecKind::Shell { line, program } => Exec::Shell { line, program },
            ProtoExecKind::Direct { stages } => Exec::Direct {
                stages: stages
                    .into_iter()
                    .map(|s| StageSpec { program: s.program, args: s.args })
                    .collect(),
            },
        };
        let spec = CommandSpec {
            exec: exec_local,
            cwd,
            capture_limit: capture_limit_bytes,
            spill_path: None,
            stdin_data,
        };
        let mut h = shuma_exec::run(&spec);
        let killer = h.killer();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<RunEvent>();
        std::thread::spawn(move || {
            while let Some(ev) = h.next_event() {
                if tx.send(ev).is_err() {
                    return;
                }
            }
        });
        let mut byte = [0u8; 1];
        loop {
            tokio::select! {
                biased;
                ev = rx.recv() => {
                    let Some(ev) = ev else { break };
                    let terminal = matches!(ev, RunEvent::Exited(_) | RunEvent::Failed(_));
                    let resp = match ev {
                        RunEvent::Stdout(l) => Response::ExecStdout(l),
                        RunEvent::Stderr(l) => Response::ExecStderr(l),
                        RunEvent::Truncated => Response::ExecTruncated,
                        RunEvent::Spilled(p) => Response::ExecSpilled(p),
                        RunEvent::Exited(c) => Response::ExecExited(c),
                        RunEvent::Failed(m) => Response::ExecFailed(m),
                    };
                    if write_frame(&mut stream, &resp).await.is_err() {
                        killer.kill();
                        return;
                    }
                    if terminal {
                        break;
                    }
                }
                res = tokio::io::AsyncReadExt::read_exact(&mut stream, &mut byte) => {
                    let _ = res;
                    killer.kill();
                    while let Some(ev) = rx.recv().await {
                        if matches!(ev, RunEvent::Exited(_) | RunEvent::Failed(_)) {
                            break;
                        }
                    }
                    break;
                }
            }
        }
    }

    /// Arranca un mini servidor en `path` que sirve UNA conexión y se
    /// apaga. Devuelve cuando termina la atención.
    async fn one_shot_server(path: std::path::PathBuf) {
        let listener = tokio::net::UnixListener::bind(&path).expect("bind");
        let (stream, _) = listener.accept().await.expect("accept");
        serve_exec_stream(stream).await;
        let _ = std::fs::remove_file(&path);
    }

    fn temp_socket() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "shuma-remote-test-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn echo_round_trips_through_remote_client() {
        let sock = temp_socket();
        let sock_for_server = sock.clone();
        let server = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(one_shot_server(sock_for_server));
        });
        // Esperamos a que el server bindee.
        while !sock.exists() {
            std::thread::sleep(Duration::from_millis(10));
        }
        let spec = CommandSpec::direct(
            vec![StageSpec {
                program: "echo".into(),
                args: vec!["hola".into(), "remoto".into()],
            }],
            ".",
        );
        let mut h = run(&spec, &sock).expect("run remote");
        let mut got_line = String::new();
        while let Some(ev) = h.next_event() {
            if let RunEvent::Stdout(l) = ev {
                got_line = l;
            }
        }
        server.join().unwrap();
        assert_eq!(got_line, "hola remoto");
        assert!(h.is_finished());
    }

    #[test]
    fn kill_propagates_to_remote_process() {
        let sock = temp_socket();
        let sock_for_server = sock.clone();
        let server = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(one_shot_server(sock_for_server));
        });
        while !sock.exists() {
            std::thread::sleep(Duration::from_millis(10));
        }
        let spec = CommandSpec::direct(
            vec![StageSpec { program: "sleep".into(), args: vec!["30".into()] }],
            ".",
        );
        let mut h = run(&spec, &sock).expect("run remote");
        // Le damos tiempo a arrancar; luego pedimos kill.
        std::thread::sleep(Duration::from_millis(150));
        h.kill();
        // El stream se cierra, el server mata al hijo, el next_event sale
        // (puede salir como None si no llega ningún terminal — el server
        // ya cerró antes de poder enviarlo).
        let started = std::time::Instant::now();
        while h.next_event().is_some() {
            if started.elapsed() > Duration::from_secs(5) {
                panic!("kill no terminó el stream en 5s");
            }
        }
        assert!(h.is_finished());
        server.join().unwrap();
    }

    #[test]
    fn connect_error_surfaces_to_caller() {
        let no_such = std::path::PathBuf::from("/tmp/shuma-no-such-socket.sock");
        let _ = std::fs::remove_file(&no_such);
        let spec = CommandSpec::direct(
            vec![StageSpec { program: "true".into(), args: vec![] }],
            ".",
        );
        match run(&spec, &no_such) {
            Err(RemoteExecError::Connect(_, _)) => {}
            Err(e) => panic!("variante de error inesperada: {e:?}"),
            Ok(_) => panic!("debería fallar al conectar a un socket inexistente"),
        }
    }

    // ----- Tests del path TCP autenticado (Noise XK) -----

    /// Espejo de `serve_exec_stream` para FramedChannel — sirve un único
    /// ExecStream sobre el canal cifrado. Replica la forma del
    /// `handle_exec_stream_enc` del daemon pero in-process para el test.
    async fn serve_one_exec_enc(
        mut ch: shuma_link::FramedChannel<tokio::net::TcpStream>,
    ) {
        let req: Request = ch.recv_postcard().await.expect("recv request");
        let Request::ExecStream { cwd, exec, capture_limit_bytes, stdin_data } = req else {
            panic!("esperaba ExecStream");
        };
        let exec_local = match exec {
            ProtoExecKind::Shell { line, program } => Exec::Shell { line, program },
            ProtoExecKind::Direct { stages } => Exec::Direct {
                stages: stages
                    .into_iter()
                    .map(|s| StageSpec { program: s.program, args: s.args })
                    .collect(),
            },
        };
        let spec = CommandSpec {
            exec: exec_local,
            cwd,
            capture_limit: capture_limit_bytes,
            spill_path: None,
            stdin_data,
        };
        let mut h = shuma_exec::run(&spec);
        let killer = h.killer();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<RunEvent>();
        std::thread::spawn(move || {
            while let Some(ev) = h.next_event() {
                if tx.send(ev).is_err() {
                    return;
                }
            }
        });
        while let Some(ev) = rx.recv().await {
            let terminal = matches!(ev, RunEvent::Exited(_) | RunEvent::Failed(_));
            let resp = match ev {
                RunEvent::Stdout(l) => Response::ExecStdout(l),
                RunEvent::Stderr(l) => Response::ExecStderr(l),
                RunEvent::Truncated => Response::ExecTruncated,
                RunEvent::Spilled(p) => Response::ExecSpilled(p),
                RunEvent::Exited(c) => Response::ExecExited(c),
                RunEvent::Failed(m) => Response::ExecFailed(m),
            };
            if ch.send_postcard(&resp).await.is_err() {
                killer.kill();
                return;
            }
            if terminal {
                break;
            }
        }
    }

    #[test]
    fn echo_round_trips_through_encrypted_tcp_client() {
        // Bind a localhost con puerto efímero para evitar choque.
        let server_kp = shuma_link::Keypair::generate().unwrap();
        let client_kp = shuma_link::Keypair::generate().unwrap();
        let server_pub = server_kp.public();

        // Server-thread con su propio runtime para no contender el del
        // cliente (que vive dentro del hilo de `run_tcp`).
        let server_kp2 = server_kp.clone();
        let (addr_tx, addr_rx) = std::sync::mpsc::channel::<String>();
        let server_thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap().to_string();
                addr_tx.send(addr).unwrap();
                let (tcp, _) = listener.accept().await.unwrap();
                let (ch, _peer) = shuma_link::server_handshake(tcp, &server_kp2).await.unwrap();
                serve_one_exec_enc(ch).await;
            });
        });
        let addr = addr_rx.recv().unwrap();
        let spec = CommandSpec::direct(
            vec![StageSpec { program: "echo".into(), args: vec!["hola".into(), "cifrado".into()] }],
            ".",
        );
        let mut h = run_tcp(&spec, &addr, client_kp, server_pub).unwrap();
        let mut got = String::new();
        while let Some(ev) = h.next_event() {
            if let RunEvent::Stdout(l) = ev {
                got = l;
            }
        }
        server_thread.join().unwrap();
        assert_eq!(got, "hola cifrado");
        assert!(h.is_finished());
    }

    #[test]
    fn wrong_server_pubkey_surfaces_as_failed_event() {
        // El cliente espera la pubkey de un server "legítimo", pero el
        // que responde es otro. El handshake debe fallar y eso llega al
        // shell como un RunEvent::Failed, NO como un panic.
        let real_server = shuma_link::Keypair::generate().unwrap();
        let attacker = shuma_link::Keypair::generate().unwrap();
        let client = shuma_link::Keypair::generate().unwrap();

        let (addr_tx, addr_rx) = std::sync::mpsc::channel::<String>();
        let server_thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                addr_tx.send(listener.local_addr().unwrap().to_string()).unwrap();
                let (tcp, _) = listener.accept().await.unwrap();
                // El "atacante" se identifica con su propia keypair.
                let _ = shuma_link::server_handshake(tcp, &attacker).await;
            });
        });
        let addr = addr_rx.recv().unwrap();
        let spec = CommandSpec::direct(
            vec![StageSpec { program: "true".into(), args: vec![] }],
            ".",
        );
        let mut h = run_tcp(&spec, &addr, client, real_server.public()).unwrap();
        let mut saw_failed = false;
        while let Some(ev) = h.next_event() {
            if matches!(ev, RunEvent::Failed(_)) {
                saw_failed = true;
            }
        }
        server_thread.join().ok();
        assert!(saw_failed, "se esperaba RunEvent::Failed por pubkey errónea");
    }
}
