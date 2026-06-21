//! Registro de sesiones PTY persistentes (tmux-like) del daemon.
//!
//! Una sesión es un proceso bajo pseudo-terminal cuyo ciclo de vida está
//! **desacoplado de cualquier conexión**: el cliente se adjunta y se
//! desadjunta libremente; cerrar la conexión NO mata el proceso. El
//! proceso sólo muere si termina solo o se le manda `PtyKill`. (No
//! persiste a reinicio del daemon — igual que tmux pierde sus sesiones si
//! matas el servidor.)
//!
//! Diseño:
//! - Un hilo de fondo por sesión drena `RunHandle::next_event()` (API
//!   bloqueante de `shuma-exec`) hacia dos sumideros: el **ring** (los
//!   últimos `RING_CAP` bytes, para repintar a quien (re)adjunta) y un
//!   canal **broadcast** (la salida en vivo a los clientes adjuntos).
//! - `Shared` (buffer + alive + exit) está bajo un único `Mutex`, y el
//!   drain hace *push/marcar-muerto + broadcast* mientras lo tiene tomado.
//!   Un cliente que se adjunta toma ese mismo lock para suscribirse y
//!   sacar el snapshot a la vez → ni pierde ni duplica bytes en la
//!   frontera scrollback↔vivo.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use shuma_exec::RunEvent;
use shuma_protocol::PtySessionInfo;
use tokio::sync::broadcast;
use ulid::Ulid;

/// Bytes de scrollback retenidos por sesión. Un terminal típico cabe de
/// sobra; al exceder, se descartan los más viejos (anillo).
const RING_CAP: usize = 256 * 1024;

/// Capacidad del broadcast de salida por sesión. Si un cliente adjunto se
/// atrasa más de esto, recibe `Lagged` y le repintamos el scrollback
/// completo en vez de arrastrar bytes perdidos (que corromperían la
/// pantalla vt100).
const BROADCAST_CAP: usize = 1024;

/// Un frame de salida de la sesión hacia los clientes adjuntos.
#[derive(Clone)]
pub enum SessionEvent {
    /// Bytes crudos del terminal. `Arc` para no clonar el buffer una vez
    /// por subscriber del broadcast.
    Bytes(Arc<Vec<u8>>),
    /// La sesión terminó con este código.
    Exited(i32),
}

/// Estado mutable protegido por un único lock: el ring de scrollback y el
/// estado de vida. Tenerlos juntos hace que *acumular salida*, *marcar
/// muerto* y *suscribirse* sean operaciones bien ordenadas entre sí.
struct Shared {
    buf: VecDeque<u8>,
    alive: bool,
    exit: Option<i32>,
}

impl Shared {
    fn push(&mut self, bytes: &[u8]) {
        self.buf.extend(bytes.iter().copied());
        while self.buf.len() > RING_CAP {
            self.buf.pop_front();
        }
    }
    fn snapshot(&self) -> Vec<u8> {
        self.buf.iter().copied().collect()
    }
}

/// Metadatos inmutables de una sesión (lo que se reporta en `PtyList`).
struct Meta {
    label: String,
    program: String,
    args: Vec<String>,
    cwd: String,
    rows: u16,
    cols: u16,
    created_unix_ms: u64,
}

/// Una sesión PTY persistente. El registro guarda los handles de control
/// (clonables y desacoplados del lock de eventos); el hilo de drenado
/// mantiene `shared` y `tx` al día.
pub struct PtySession {
    meta: Meta,
    control: shuma_exec::PtyControl,
    killer: shuma_exec::Killer,
    shared: Arc<Mutex<Shared>>,
    tx: broadcast::Sender<SessionEvent>,
}

impl PtySession {
    /// Reescala el PTY al tamaño del cliente que se adjunta.
    pub fn resize(&self, rows: u16, cols: u16) {
        self.control.resize(rows, cols);
    }

    /// Reenvía teclas al PTY.
    pub fn write_input(&self, bytes: Vec<u8>) {
        self.control.write_input(bytes);
    }

    /// Se suscribe al stream en vivo y saca el scrollback **de forma
    /// atómica** respecto al drenado: ambos bajo el mismo lock, así no hay
    /// hueco ni solape en la frontera. Devuelve el receiver, el snapshot
    /// del scrollback, y —si la sesión ya murió— su código de salida (en
    /// cuyo caso el `Exited` ya se emitió antes de esta suscripción y hay
    /// que sintetizarlo, porque el receiver no lo verá).
    pub fn attach(&self) -> Attachment {
        let s = self.shared.lock().expect("pty shared lock");
        let rx = self.tx.subscribe();
        let scrollback = s.snapshot();
        let exited = if s.alive { None } else { Some(s.exit.unwrap_or(-1)) };
        Attachment { rx, scrollback, exited }
    }

    /// Snapshot del scrollback (para repintar tras un `Lagged`).
    pub fn scrollback(&self) -> Vec<u8> {
        self.shared.lock().expect("pty shared lock").snapshot()
    }

    fn info(&self, session: Ulid) -> PtySessionInfo {
        let s = self.shared.lock().expect("pty shared lock");
        PtySessionInfo {
            session,
            label: self.meta.label.clone(),
            program: self.meta.program.clone(),
            args: self.meta.args.clone(),
            cwd: self.meta.cwd.clone(),
            rows: self.meta.rows,
            cols: self.meta.cols,
            alive: s.alive,
            exit_code: s.exit,
            created_unix_ms: self.meta.created_unix_ms,
            // `-1`: el sender propio del registro no cuenta como adjunto.
            attached: self.tx.receiver_count() as u32,
        }
    }
}

/// Resultado de adjuntarse a una sesión.
pub struct Attachment {
    pub rx: broadcast::Receiver<SessionEvent>,
    pub scrollback: Vec<u8>,
    /// `Some(code)` si la sesión ya estaba muerta al adjuntarse — el
    /// llamador debe emitir el `ExecExited(code)` él mismo.
    pub exited: Option<i32>,
}

/// Registro global de sesiones PTY del daemon.
#[derive(Default)]
pub struct PtyRegistry {
    sessions: Mutex<HashMap<Ulid, Arc<PtySession>>>,
}

impl PtyRegistry {
    /// Crea y registra una sesión: spawnea el proceso bajo PTY y arranca
    /// el hilo de drenado. Devuelve el id.
    pub fn spawn(
        &self,
        cwd: String,
        program: String,
        args: Vec<String>,
        rows: u16,
        cols: u16,
        label: String,
    ) -> Ulid {
        // El id de la sesión se siembra como `SHUMA_SESSION` en el entorno del
        // PTY (lo hereda el shell, claude y sus hooks), para que un aviso de
        // hook pueda enlazar a ESTA sesión exacta. Envolvemos con `/usr/bin/
        // env` en vez de tocar `CommandSpec`: el `Meta` guarda el comando
        // original (la lista queda limpia), pero el proceso recibe la env.
        let id = Ulid::new();
        let mut wrapped = Vec::with_capacity(args.len() + 2);
        wrapped.push(format!("SHUMA_SESSION={id}"));
        wrapped.push(program.clone());
        wrapped.extend(args.iter().cloned());
        let spec = shuma_exec::CommandSpec {
            exec: shuma_exec::Exec::Pty {
                program: "/usr/bin/env".to_string(),
                args: wrapped,
                cols,
                rows,
            },
            cwd: cwd.clone(),
            capture_limit: 0,
            spill_path: None,
            stdin_data: None,
            capture_stages: false,
        };
        let mut handle = shuma_exec::run(&spec);
        let killer = handle.killer();
        let control = handle.pty_control();

        let shared = Arc::new(Mutex::new(Shared {
            buf: VecDeque::new(),
            alive: true,
            exit: None,
        }));
        let (tx, _rx) = broadcast::channel(BROADCAST_CAP);

        // Hilo de drenado: bloquea en `next_event()` y reparte cada evento
        // al ring + broadcast, siempre bajo el lock de `shared` para
        // ordenar correctamente respecto a quien se suscribe.
        {
            let shared = Arc::clone(&shared);
            let tx = tx.clone();
            std::thread::spawn(move || {
                while let Some(ev) = handle.next_event() {
                    match ev {
                        RunEvent::Bytes(b) => {
                            let mut s = shared.lock().expect("pty shared lock");
                            s.push(&b);
                            let _ = tx.send(SessionEvent::Bytes(Arc::new(b)));
                        }
                        // Un PTY captura a su pantalla, no por líneas; si
                        // aún así llegan, las tratamos como bytes crudos.
                        RunEvent::Stdout(l) | RunEvent::Stderr(l) => {
                            let bytes = l.into_bytes();
                            let mut s = shared.lock().expect("pty shared lock");
                            s.push(&bytes);
                            let _ = tx.send(SessionEvent::Bytes(Arc::new(bytes)));
                        }
                        RunEvent::StageStdout { line, .. } => {
                            let bytes = line.into_bytes();
                            let mut s = shared.lock().expect("pty shared lock");
                            s.push(&bytes);
                            let _ = tx.send(SessionEvent::Bytes(Arc::new(bytes)));
                        }
                        RunEvent::Exited(c) => {
                            let mut s = shared.lock().expect("pty shared lock");
                            s.alive = false;
                            s.exit = Some(c);
                            let _ = tx.send(SessionEvent::Exited(c));
                            break;
                        }
                        RunEvent::Failed(m) => {
                            let bytes = m.into_bytes();
                            let mut s = shared.lock().expect("pty shared lock");
                            s.push(&bytes);
                            let _ = tx.send(SessionEvent::Bytes(Arc::new(bytes)));
                            s.alive = false;
                            s.exit = Some(-1);
                            let _ = tx.send(SessionEvent::Exited(-1));
                            break;
                        }
                        RunEvent::Truncated | RunEvent::Spilled(_) => {}
                    }
                }
                // Salvaguarda: si `next_event` devuelve `None` sin terminal
                // explícito (canal cerrado), marcamos muerta la sesión para
                // que los que se adjunten después no queden colgados.
                let mut s = shared.lock().expect("pty shared lock");
                if s.alive {
                    s.alive = false;
                    s.exit = s.exit.or(Some(-1));
                    let _ = tx.send(SessionEvent::Exited(s.exit.unwrap_or(-1)));
                }
            });
        }

        let label = if label.trim().is_empty() {
            program.clone()
        } else {
            label
        };
        let created_unix_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let session = Arc::new(PtySession {
            meta: Meta {
                label,
                program,
                args,
                cwd,
                rows,
                cols,
                created_unix_ms,
            },
            control,
            killer,
            shared,
            tx,
        });

        self.sessions
            .lock()
            .expect("pty registry lock")
            .insert(id, session);
        id
    }

    /// Handle de una sesión por id (para adjuntarse), si existe.
    pub fn get(&self, id: Ulid) -> Option<Arc<PtySession>> {
        self.sessions
            .lock()
            .expect("pty registry lock")
            .get(&id)
            .cloned()
    }

    /// Todas las sesiones registradas, ordenadas por antigüedad (más
    /// nuevas al final).
    pub fn list(&self) -> Vec<PtySessionInfo> {
        let map = self.sessions.lock().expect("pty registry lock");
        let mut out: Vec<PtySessionInfo> = map.iter().map(|(id, s)| s.info(*id)).collect();
        out.sort_by_key(|i| i.created_unix_ms);
        out
    }

    /// Mata (o reapea, si ya murió) y quita del registro. `false` si no
    /// existía. Tras quitarla, el `Arc` muere cuando los clientes adjuntos
    /// se desadjunten; el SIGKILL hace que el hilo de drenado vea `Exited`
    /// y termine solo.
    pub fn kill(&self, id: Ulid) -> bool {
        let removed = self
            .sessions
            .lock()
            .expect("pty registry lock")
            .remove(&id);
        match removed {
            Some(session) => {
                session.killer.kill();
                true
            }
            None => false,
        }
    }
}
