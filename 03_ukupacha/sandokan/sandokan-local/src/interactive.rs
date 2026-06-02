//! Sesiones interactivas (PTY) del `LocalEngine` — la base del "Modo Tmux".
//!
//! Una sesión interactiva es una Card encarnada **aislada** (vía
//! `arje-incarnate`) cuyo stdio está atado a un PTY que el engine retiene.
//! El engine lee el master en un hilo, acumula un scrollback acotado y lo
//! difunde por un canal broadcast. Los clientes hacen `attach(card_id)`:
//! reciben el scrollback ya emitido (replay) + un stream de bytes vivos, y
//! pueden escribir input. Soltar la `Attachment` = detach; la sesión sigue
//! viva en el engine, lista para re-attach (otro cliente, u otra ventana).
//!
//! Sin TIOCSCTTY todavía: el shell hace `setsid` pero no toma el PTY como
//! controlling terminal, así que el job control fino (Ctrl-C al foreground
//! group) es un refinamiento futuro. La E/S de línea y el re-attach ya
//! funcionan.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use arje_incarnate::{ChildPreExec, ChildSetup, ChildStdio, Incarnator};
use nix::pty::{openpty, OpenptyResult, Winsize};
use sandokan_core::{EngineError, ExecHandle, Intent, InteractiveEngine, PtySize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use ulid::Ulid;

use crate::{Entity, LocalEngine};
use sandokan_lifecycle::LifecycleState;

/// Bytes máximos de scrollback retenidos por sesión (replay al attach).
const SCROLLBACK_CAP: usize = 64 * 1024;
/// Capacidad del canal broadcast (chunks en vuelo antes de Lagged).
const LIVE_CHANNEL_CAP: usize = 2048;

/// Un enganche a una sesión viva: el scrollback ya emitido + un stream de
/// los bytes vivos desde el momento del attach. Soltar esto = detach.
pub struct Attachment {
    /// Bytes ya emitidos por la sesión hasta el attach (para repintar la
    /// pantalla del cliente que se engancha tarde).
    pub scrollback: Vec<u8>,
    /// Stream de bytes vivos. `RecvError::Closed` = la sesión terminó.
    pub live: broadcast::Receiver<Vec<u8>>,
}

/// Ring de scrollback: conserva los últimos `cap` bytes.
struct Scrollback {
    buf: Vec<u8>,
    cap: usize,
}

impl Scrollback {
    fn new(cap: usize) -> Self {
        Self {
            buf: Vec::new(),
            cap,
        }
    }
    fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
        if self.buf.len() > self.cap {
            let cut = self.buf.len() - self.cap;
            self.buf.drain(0..cut);
        }
    }
    fn snapshot(&self) -> Vec<u8> {
        self.buf.clone()
    }
}

/// Estado retenido de una sesión interactiva viva.
pub(crate) struct Session {
    /// Lado de escritura del master (input del usuario → PTY). `Arc` porque
    /// el servidor de socket también escribe input desde los clientes.
    write: Arc<Mutex<File>>,
    scrollback: Arc<Mutex<Scrollback>>,
    tx: broadcast::Sender<Vec<u8>>,
    /// Socket canónico de esta sesión (`<run_dir>/<card_id>.sock`).
    sock_path: PathBuf,
    /// Tarea del servidor de socket. Se aborta al dropear la sesión.
    server: JoinHandle<()>,
    /// Spec re-corrible (Intent + tamaño) para la re-hidratación del Model 1:
    /// al reiniciar el daemon, la sesión se relanza con esto y reaparece su
    /// `<card_id>.sock` con el mismo id → el front re-ataja sin enterarse.
    spec: SessionSnapshot,
}

/// Snapshot re-corrible de una sesión interactiva: su `Intent` (que ya lleva
/// el `card_id` en `card.id`) y el tamaño del PTY. Es lo que persiste el
/// daemon para re-hidratar sus sesiones tras un reinicio (Model 1: relanza un
/// shell fresco, igual que tmux pierde sesiones si muere el server).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionSnapshot {
    pub intent: Intent,
    pub size: PtySize,
}

/// El estado interactivo persistible del engine: todas sus sesiones vivas.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct EngineSnapshot {
    pub sessions: Vec<SessionSnapshot>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.server.abort();
        let _ = std::fs::remove_file(&self.sock_path);
    }
}

impl Session {
    fn attach(&self) -> Attachment {
        Attachment {
            scrollback: self.scrollback.lock().expect("scrollback lock").snapshot(),
            live: self.tx.subscribe(),
        }
    }

    fn write_input(&self, bytes: &[u8]) -> Result<(), EngineError> {
        let mut w = self.write.lock().expect("pty write lock");
        w.write_all(bytes)
            .and_then(|_| w.flush())
            .map_err(|e| EngineError::Transport(format!("write pty: {e}")))
    }

    fn resize(&self, size: PtySize) -> Result<(), EngineError> {
        let fd = self.write.lock().expect("pty write lock").as_raw_fd();
        let ws = Winsize {
            ws_row: size.rows,
            ws_col: size.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let rc = unsafe { nix::libc::ioctl(fd, nix::libc::TIOCSWINSZ, &ws) };
        if rc == 0 {
            Ok(())
        } else {
            Err(EngineError::Transport("ioctl TIOCSWINSZ".into()))
        }
    }
}

/// Crea un PTY, encarna la Card aislada con su slave como stdio (+ setsid),
/// retiene el master y arranca el hilo lector. Devuelve la sesión + el pid.
fn spawn_pty(
    intent: &Intent,
    cfg: arje_incarnate::IncarnatorConfig,
    size: PtySize,
    sock_path: PathBuf,
) -> Result<(Session, i32), EngineError> {
    let card = &intent.card;
    let ws = Winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let OpenptyResult { master, slave } =
        openpty(Some(&ws), None).map_err(|e| EngineError::Incarnate(format!("openpty: {e}")))?;

    // El slave es stdin/stdout/stderr del hijo aislado. Cedemos su propiedad
    // con `into_raw_fd` (sin Drop): arje-incarnate lo dupea en el hijo y lo
    // cierra en el padre tras encarnar, así el master ve EOF cuando el hijo
    // muere. (Cerrarlo también nosotros sería doble-close → abort.)
    let slave_raw = slave.into_raw_fd();
    let stdio = ChildStdio {
        stdin_fd: Some(slave_raw),
        stdout_fd: Some(slave_raw),
        stderr_fd: Some(slave_raw),
    };
    // NewSession + ControllingTty: el shell es session leader y toma el PTS
    // como su controlling terminal → job control real (Ctrl-C al foreground).
    let setup = ChildSetup::new()
        .with(ChildPreExec::NewSession)
        .with(ChildPreExec::ControllingTty);

    let outcome = Incarnator::new(cfg)
        .incarnate_full(card, stdio, setup)
        .map_err(|e| EngineError::Incarnate(e.to_string()))?;
    let pid = outcome.pid.as_raw();

    // Master: un fd para leer (hilo) y otro dup para escribir (sesión).
    let read_fd = master
        .try_clone()
        .map_err(|e| EngineError::Transport(format!("dup pty master: {e}")))?;
    let read_file = File::from(read_fd);
    let write_file = unsafe { File::from_raw_fd(master.into_raw_fd()) };

    let scrollback = Arc::new(Mutex::new(Scrollback::new(SCROLLBACK_CAP)));
    let write = Arc::new(Mutex::new(write_file));
    let (tx, _rx) = broadcast::channel::<Vec<u8>>(LIVE_CHANNEL_CAP);

    // Hilo lector: master → scrollback + broadcast. Termina al EOF del PTY.
    let sb = scrollback.clone();
    let txc = tx.clone();
    std::thread::Builder::new()
        .name(format!("sandokan-pty-{pid}"))
        .spawn(move || {
            let mut f = read_file;
            let mut buf = [0u8; 4096];
            loop {
                match f.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let chunk = buf[..n].to_vec();
                        sb.lock().expect("scrollback lock").push(&chunk);
                        // Ok aunque no haya receivers (sesión sin attach).
                        let _ = txc.send(chunk);
                    }
                }
            }
        })
        .map_err(|e| EngineError::Transport(format!("spawn pty reader: {e}")))?;

    // Servidor de socket canónico: el front se conecta a `<card_id>.sock` y
    // recibe scrollback + stream vivo, y su input va al PTY. No asume quién
    // atiende — hoy este engine, mañana un holder por sesión (Model 2).
    let server = spawn_socket_server(sock_path.clone(), scrollback.clone(), tx.clone(), write.clone());

    Ok((
        Session {
            write,
            scrollback,
            tx,
            sock_path,
            server,
            spec: SessionSnapshot {
                intent: intent.clone(),
                size,
            },
        },
        pid,
    ))
}

/// Sirve la sesión por un Unix socket: cada cliente recibe el scrollback
/// (replay) + el stream vivo; lo que el cliente escribe va al PTY. Múltiples
/// clientes conviven (espejo de pantalla). Es el contrato estable con el
/// front — agnóstico de si detrás hay un engine in-process o un holder.
fn spawn_socket_server(
    path: PathBuf,
    scrollback: Arc<Mutex<Scrollback>>,
    tx: broadcast::Sender<Vec<u8>>,
    write: Arc<Mutex<File>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let _ = std::fs::remove_file(&path); // limpiar un socket stale
        let listener = match UnixListener::bind(&path) {
            Ok(l) => l,
            Err(_) => return,
        };
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(x) => x,
                Err(_) => break,
            };
            let sb = scrollback.clone();
            let mut rx = tx.subscribe();
            let w = write.clone();
            tokio::spawn(async move {
                let (mut rd, mut wr) = stream.into_split();
                // Replay del scrollback (snapshot sin sostener el lock en await).
                let snap = sb.lock().expect("scrollback lock").snapshot();
                if wr.write_all(&snap).await.is_err() {
                    return;
                }
                // Vivo → cliente.
                let live = tokio::spawn(async move {
                    loop {
                        match rx.recv().await {
                            Ok(chunk) => {
                                if wr.write_all(&chunk).await.is_err() {
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(_) => break, // sesión terminó
                        }
                    }
                });
                // Cliente → PTY (input). Lock sólo para el write sync.
                let mut buf = [0u8; 4096];
                loop {
                    match rd.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let mut g = w.lock().expect("pty write lock");
                            let _ = g.write_all(&buf[..n]);
                            let _ = g.flush();
                        }
                    }
                }
                live.abort();
            });
        }
    })
}

#[async_trait::async_trait]
impl InteractiveEngine for LocalEngine {
    /// Encarna una Card **interactiva**: aislada (como `run`) pero atada a un
    /// PTY que el engine retiene para `attach` posterior. Aparece en
    /// `list`/`status`/`stop` como cualquier entidad.
    async fn run_interactive(
        &self,
        intent: Intent,
        size: PtySize,
    ) -> Result<ExecHandle, EngineError> {
        let card_id = intent.card_id();
        let label = intent.card.label.clone();

        let mut cfg = self.base_cfg.clone();
        cfg.extra_env.extend(intent.context.env.clone());

        let sock_path = self.session_socket_path(card_id);
        if let Some(parent) = sock_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| EngineError::Transport(format!("mkdir run_dir: {e}")))?;
        }
        let (session, pid) = spawn_pty(&intent, cfg, size, sock_path)?;

        let handle = ExecHandle {
            card_id,
            label,
            started_at: SystemTime::now(),
        };
        self.registry.lock().expect("registry lock").insert(
            card_id,
            Entity {
                handle: handle.clone(),
                pid,
                state: LifecycleState::Running,
            },
        );
        self.sessions
            .lock()
            .expect("sessions lock")
            .insert(card_id, Arc::new(session));
        self.autosave(); // persistir el set de sesiones (Model 1)
        Ok(handle)
    }

    fn session_socket_path(&self, card_id: Ulid) -> PathBuf {
        self.run_dir.join(format!("{card_id}.sock"))
    }
}

impl LocalEngine {
    /// Engancha a una sesión interactiva viva por `card_id`: devuelve el
    /// scrollback + un stream de bytes vivos. Múltiples attaches conviven
    /// (espejo de pantalla). Soltar el `Attachment` = detach silencioso.
    pub async fn attach(&self, card_id: Ulid) -> Result<Attachment, EngineError> {
        let s = self.session_arc(card_id)?;
        Ok(s.attach())
    }

    /// Escribe input (teclas) al PTY de una sesión viva.
    pub async fn write_input(&self, card_id: Ulid, bytes: &[u8]) -> Result<(), EngineError> {
        self.session_arc(card_id)?.write_input(bytes)
    }

    /// Redimensiona el PTY de una sesión (al cambiar de tamaño la ventana).
    pub async fn resize(&self, card_id: Ulid, size: PtySize) -> Result<(), EngineError> {
        self.session_arc(card_id)?.resize(size)
    }

    fn session_arc(&self, card_id: Ulid) -> Result<Arc<Session>, EngineError> {
        self.sessions
            .lock()
            .expect("sessions lock")
            .get(&card_id)
            .cloned()
            .ok_or(EngineError::NotFound(card_id))
    }

    /// Olvida la sesión interactiva (libera el master PTY). El `stop` del
    /// trait `Engine` la llama tras matar el proceso.
    pub(crate) fn drop_session(&self, card_id: Ulid) {
        self.sessions.lock().expect("sessions lock").remove(&card_id);
        self.autosave();
    }

    /// Persiste el snapshot interactivo si hay `snapshot_path` configurado
    /// (best-effort). Llamado tras cada alta/baja de sesión para que el
    /// archivo refleje el estado vivo (re-hidratación al reiniciar).
    pub(crate) fn autosave(&self) {
        if let Some(path) = self.snapshot_path.clone() {
            let _ = self.save_snapshot(&path);
        }
    }
}

/// El mapa de sesiones vive en `LocalEngine`; este alias lo nombra en lib.rs.
pub(crate) type SessionMap = HashMap<Ulid, Arc<Session>>;

// ---------------------------------------------------------------------------
// Re-hidratación (Model 1): persistir specs de sesiones vivas y relanzarlas
// al reiniciar el daemon. El shell es fresco, pero conserva su card_id → su
// `<card_id>.sock` reaparece y el front re-ataja sin enterarse.
// ---------------------------------------------------------------------------

impl LocalEngine {
    /// Snapshot de las sesiones interactivas vivas, para persistir.
    pub fn interactive_snapshot(&self) -> EngineSnapshot {
        let sessions = self
            .sessions
            .lock()
            .expect("sessions lock")
            .values()
            .map(|s| s.spec.clone())
            .collect();
        EngineSnapshot { sessions }
    }

    /// Re-hidrata sesiones desde un snapshot: relanza cada una (shell fresco)
    /// con su `card_id` original. Devuelve un handle (o error) por sesión.
    pub async fn rehydrate(&self, snap: EngineSnapshot) -> Vec<Result<ExecHandle, EngineError>> {
        let mut out = Vec::with_capacity(snap.sessions.len());
        for s in snap.sessions {
            out.push(self.run_interactive(s.intent, s.size).await);
        }
        out
    }

    /// Persiste el snapshot interactivo a un archivo JSON (crea el dir padre).
    pub fn save_snapshot(&self, path: &std::path::Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.interactive_snapshot())
            .map_err(std::io::Error::other)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, bytes)
    }

    /// Carga un snapshot JSON y re-hidrata sus sesiones. Archivo ausente =
    /// no-op (devuelve 0). Devuelve cuántas sesiones se relanzaron.
    pub async fn restore_snapshot(&self, path: &std::path::Path) -> std::io::Result<usize> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let snap: EngineSnapshot = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        let n = snap.sessions.len();
        self.rehydrate(snap).await;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use card_core::{Card, NamespaceSet, Payload};
    use sandokan_core::{Engine, InteractiveEngine};
    use std::time::Duration;

    fn isolated_sh() -> Card {
        let mut c = Card::new("term");
        c.payload = Payload::Native {
            exec: "/bin/sh".into(),
            argv: vec![],
            envp: vec![],
        };
        c.soma.namespaces = NamespaceSet {
            user: true,
            pid: true,
            mount: true,
            uts: true,
            ipc: true,
            net: false,
            cgroup: false,
        };
        c
    }

    /// Lee del stream vivo hasta ver `needle` o vencer `ms`.
    async fn live_until(rx: &mut broadcast::Receiver<Vec<u8>>, needle: &str, ms: u64) -> bool {
        let mut acc = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(chunk)) => {
                    acc.extend_from_slice(&chunk);
                    if String::from_utf8_lossy(&acc).contains(needle) {
                        return true;
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                _ => return false,
            }
        }
    }

    #[tokio::test]
    async fn interactive_session_attaches_streams_and_replays_scrollback() {
        let e = LocalEngine::new();
        let card = isolated_sh();
        let id = card.id;
        e.run_interactive(Intent::new(card), PtySize::default())
            .await
            .expect("run_interactive");

        // Attach #1: escribir un comando y verlo en el stream vivo.
        let mut att = e.attach(id).await.expect("attach");
        e.write_input(id, b"echo SANDOKAN_OK\n")
            .await
            .expect("write_input");
        assert!(
            live_until(&mut att.live, "SANDOKAN_OK", 4000).await,
            "el marcador no apareció en el stream vivo"
        );
        drop(att); // detach: la sesión sigue viva

        // Re-attach: el scrollback retuvo la salida previa (Modo Tmux).
        let att2 = e.attach(id).await.expect("re-attach");
        let sb = String::from_utf8_lossy(&att2.scrollback);
        assert!(
            sb.contains("SANDOKAN_OK"),
            "el scrollback no retuvo la sesión: {sb:?}"
        );

        // Cierre ordenado.
        e.write_input(id, b"exit\n").await.ok();
        tokio::time::sleep(Duration::from_millis(300)).await;
        e.stop(id, Duration::ZERO).await.ok();
    }

    #[tokio::test]
    async fn interactive_session_has_a_controlling_tty() {
        // `echo ... > /dev/tty` sólo funciona si el proceso TIENE un
        // controlling terminal (TIOCSCTTY aplicado). Sin él, abrir /dev/tty
        // da ENXIO y el comando no produce salida.
        let e = LocalEngine::new();
        let card = isolated_sh();
        let id = card.id;
        e.run_interactive(Intent::new(card), PtySize::default())
            .await
            .expect("run_interactive");
        let mut att = e.attach(id).await.expect("attach");
        e.write_input(id, b"echo CTTY_OK > /dev/tty\n")
            .await
            .expect("write");
        assert!(
            live_until(&mut att.live, "CTTY_OK", 4000).await,
            "el shell no tiene controlling tty (/dev/tty falló)"
        );
        e.stop(id, Duration::ZERO).await.ok();
    }

    #[tokio::test]
    async fn attach_to_unknown_session_is_not_found() {
        let e = LocalEngine::new();
        assert!(matches!(
            e.attach(Ulid::new()).await,
            Err(EngineError::NotFound(_))
        ));
    }

    /// Lee del UnixStream hasta ver `needle` o vencer `ms`.
    async fn socket_until(s: &mut tokio::net::UnixStream, needle: &str, ms: u64) -> bool {
        let mut acc = Vec::new();
        let mut buf = [0u8; 4096];
        let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
        loop {
            match tokio::time::timeout_at(deadline, s.read(&mut buf)).await {
                Ok(Ok(n)) if n > 0 => {
                    acc.extend_from_slice(&buf[..n]);
                    if String::from_utf8_lossy(&acc).contains(needle) {
                        return true;
                    }
                }
                _ => return String::from_utf8_lossy(&acc).contains(needle),
            }
        }
    }

    /// El contrato con el front: la sesión se atiende por `<card_id>.sock`.
    /// Un cliente conecta, manda un comando y lee su salida; un segundo
    /// cliente que re-conecta recibe el replay del scrollback (re-attach).
    #[tokio::test]
    async fn front_attaches_over_per_card_socket() {
        use tokio::net::UnixStream;

        let dir = std::env::temp_dir().join(format!("sandokan-test-{}", Ulid::new()));
        let e = LocalEngine::with_run_dir(arje_incarnate::IncarnatorConfig::default(), dir);
        let card = isolated_sh();
        let id = card.id;
        e.run_interactive(Intent::new(card), PtySize::default())
            .await
            .expect("run_interactive");

        let path = e.session_socket_path(id);
        for _ in 0..50 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(path.exists(), "no se creó el socket por-card: {path:?}");

        // Cliente #1: comando + lectura de su salida por el socket.
        let mut c1 = UnixStream::connect(&path).await.expect("connect #1");
        c1.write_all(b"echo SOCK_OK\n").await.unwrap();
        assert!(
            socket_until(&mut c1, "SOCK_OK", 4000).await,
            "el marcador no llegó por el socket vivo"
        );
        drop(c1); // detach

        // Cliente #2 (re-attach): el replay del scrollback trae lo anterior.
        let mut c2 = UnixStream::connect(&path).await.expect("connect #2");
        assert!(
            socket_until(&mut c2, "SOCK_OK", 2000).await,
            "el re-attach no repitió el scrollback"
        );

        // Al parar, la sesión se dropea y el socket se borra.
        e.stop(id, Duration::ZERO).await.ok();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!path.exists(), "el socket debió borrarse al stop: {path:?}");
    }

    /// Re-hidratación Model 1: un engine arranca una sesión y la persiste; un
    /// engine nuevo (mismo run_dir, simulando reinicio del daemon) la restaura
    /// y la sesión revive con el MISMO card_id → su `<card_id>.sock` reaparece
    /// y responde. El front re-ataja sin enterarse del reinicio.
    #[tokio::test]
    async fn rehydrates_session_into_same_card_socket() {
        use tokio::net::UnixStream;

        let dir = std::env::temp_dir().join(format!("sandokan-rehy-{}", Ulid::new()));
        let snap_path = dir.join("snapshot.json");
        let card = isolated_sh();
        let id = card.id;

        // Engine A: arranca la sesión, persiste, y "muere" (sale del scope).
        {
            let a =
                LocalEngine::with_run_dir(arje_incarnate::IncarnatorConfig::default(), dir.clone());
            a.run_interactive(Intent::new(card), PtySize::default())
                .await
                .expect("run A");
            a.save_snapshot(&snap_path).expect("save snapshot");
            assert_eq!(a.interactive_snapshot().sessions.len(), 1);
            a.stop(id, Duration::ZERO).await.ok(); // mata + borra el socket viejo
        }

        // Engine B: mismo run_dir, restaura desde el archivo.
        let b = LocalEngine::with_run_dir(arje_incarnate::IncarnatorConfig::default(), dir.clone());
        let n = b.restore_snapshot(&snap_path).await.expect("restore");
        assert_eq!(n, 1, "debió re-hidratar exactamente 1 sesión");

        // La sesión revive con el MISMO card_id → mismo socket.
        let path = b.session_socket_path(id);
        for _ in 0..50 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            path.exists(),
            "la sesión re-hidratada no recreó su <card_id>.sock: {path:?}"
        );

        // Y funciona: comando + lectura por el socket re-creado.
        let mut c = UnixStream::connect(&path).await.expect("connect rehydrated");
        c.write_all(b"echo REHYDRATED\n").await.unwrap();
        assert!(
            socket_until(&mut c, "REHYDRATED", 4000).await,
            "la sesión re-hidratada no respondió"
        );
        b.stop(id, Duration::ZERO).await.ok();
    }
}
