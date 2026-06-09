//! `shuma-exec` — ejecución de comandos del shell con salida en streaming.
//!
//! Dos modos de ejecución, un mismo contrato de eventos:
//!
//! - [`Exec::Direct`] — brahman lanza y **conecta los procesos él mismo**:
//!   un `Command` por etapa del pipe, los pipes cableados con descriptores
//!   reales. Control total del árbol de procesos (matar todo el pipe de
//!   un golpe). Es el modo preferido.
//! - [`Exec::Shell`] — delega a un shell externo (`bash -c "<line>"`).
//!   Reservado para sintaxis que el modo directo aún no absorbe (globs,
//!   `$VAR`, redirecciones, `&&`). bash es **sólo un parser de sintaxis**,
//!   no el ejecutor por defecto.
//!
//! **Captura acotada.** [`CommandSpec::capture_limit`] topa los bytes en
//! RAM; pasado el tope, o se **descarta** ([`RunEvent::Truncated`]) o se
//! **vuelca a un archivo** si hay [`CommandSpec::spill_path`]
//! ([`RunEvent::Spilled`]). En ambos casos el pipe se sigue drenando, así
//! el proceso no se bloquea.
//!
//! **Reproceso.** [`CommandSpec::stdin_data`] alimenta un texto por la
//! entrada estándar: reprocesa la salida capturada de un comando previo
//! sin volver a correr el original.

#![forbid(unsafe_code)]

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use nix::fcntl::{splice, SpliceFFlags};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

/// Una etapa del pipe en ejecución directa: un binario y sus argumentos
/// ya resueltos (sin comillas, sin metacaracteres).
#[derive(Debug, Clone)]
pub struct StageSpec {
    pub program: String,
    pub args: Vec<String>,
}

/// Cómo ejecutar.
#[derive(Debug, Clone)]
pub enum Exec {
    /// Vía un shell externo — `program -c "<line>"`.
    Shell { line: String, program: String },
    /// Directo — brahman lanza y conecta cada etapa.
    Direct { stages: Vec<StageSpec> },
    /// Bajo un PTY (cross-platform vía `portable-pty`). Pensado para
    /// comandos **TUI fullscreen** (vim, htop, less, claude code) que
    /// detectan `isatty()` y rehúsan funcionar con pipes. Emite
    /// [`RunEvent::Bytes`] crudos en vez de `Stdout(String)` para que
    /// el frontend pueda alimentar un emulador vt100 propio.
    Pty {
        program: String,
        args: Vec<String>,
        cols: u16,
        rows: u16,
    },
}

/// Qué ejecutar y con qué política de captura.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub exec: Exec,
    pub cwd: String,
    /// Tope de captura en bytes; `0` = sin límite.
    pub capture_limit: usize,
    /// Si está, la salida que excede el tope se vuelca a este archivo.
    pub spill_path: Option<PathBuf>,
    /// Texto a alimentar por stdin — para reprocesar una salida previa.
    pub stdin_data: Option<String>,
    /// Si `true`, en un pipe `Direct` se intercepta el stdout de **cada
    /// etapa intermedia** (tee): además de alimentar a la siguiente, cada
    /// línea se emite como [`RunEvent::StageStdout`]. Permite ver el stream
    /// de cada etapa **en vivo**, sin re-ejecutar. Default `false` (sólo se
    /// captura la salida de la última etapa, como siempre).
    pub capture_stages: bool,
}

impl CommandSpec {
    /// Ejecución vía `bash -c "<line>"`.
    pub fn shell(line: impl Into<String>, cwd: impl Into<String>) -> Self {
        Self {
            exec: Exec::Shell { line: line.into(), program: "bash".into() },
            cwd: cwd.into(),
            capture_limit: 0,
            spill_path: None,
            stdin_data: None,
            capture_stages: false,
        }
    }

    /// Ejecución directa de un pipe de etapas.
    pub fn direct(stages: Vec<StageSpec>, cwd: impl Into<String>) -> Self {
        Self {
            exec: Exec::Direct { stages },
            cwd: cwd.into(),
            capture_limit: 0,
            spill_path: None,
            stdin_data: None,
            capture_stages: false,
        }
    }

    /// Activa la captura por etapa (tee) en pipes directos (encadenable).
    pub fn with_stage_capture(mut self) -> Self {
        self.capture_stages = true;
        self
    }

    /// Fija el tope de captura en bytes (encadenable).
    pub fn with_limit(mut self, bytes: usize) -> Self {
        self.capture_limit = bytes;
        self
    }

    /// Vuelca la salida excedente a `path` en vez de descartarla.
    pub fn with_spill(mut self, path: PathBuf) -> Self {
        self.spill_path = Some(path);
        self
    }

    /// Alimenta `data` por la entrada estándar del proceso (encadenable).
    pub fn with_stdin(mut self, data: impl Into<String>) -> Self {
        self.stdin_data = Some(data.into());
        self
    }
}

/// Un evento de la ejecución de un comando.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    /// Una línea de salida estándar (de la última etapa del pipe).
    Stdout(String),
    /// Una línea de stdout de una etapa **intermedia** del pipe (tee). Sólo
    /// se emite con `CommandSpec::capture_stages`. `stage` = índice 0-based
    /// de la etapa que la produjo. El front la muestra en el desplegable de
    /// esa etapa, sin re-ejecutar nada.
    StageStdout { stage: usize, line: String },
    /// Una línea de salida de error.
    Stderr(String),
    /// Un chunk de bytes crudos del PTY (sólo bajo [`Exec::Pty`]). El
    /// frontend debe alimentarlo a un emulador vt100 para renderizarlo;
    /// puede traer secuencias de cursor movement, erase, OSC, etc.
    Bytes(Vec<u8>),
    /// La captura alcanzó su tope; lo que sigue se descarta.
    Truncated,
    /// La captura alcanzó su tope; el resto se vuelca al archivo dado.
    Spilled(String),
    /// El proceso terminó con este código de salida.
    Exited(i32),
    /// El proceso no pudo siquiera lanzarse.
    Failed(String),
}

impl RunEvent {
    /// `true` si el evento cierra la ejecución (`Exited` o `Failed`).
    pub fn is_terminal(&self) -> bool {
        matches!(self, RunEvent::Exited(_) | RunEvent::Failed(_))
    }
}

/// Asa de un comando en ejecución. El consumidor la conserva y drena sus
/// eventos cuando le conviene.
pub struct RunHandle {
    rx: Receiver<RunEvent>,
    finished: bool,
    /// Los procesos, compartidos con el hilo coordinador para poder
    /// matarlos — todas las etapas de un pipe directo. Vacío para
    /// runs PTY.
    children: Arc<Mutex<Vec<Child>>>,
    /// PIDs vistos de runs PTY (los gestiona `portable-pty`; no son
    /// `std::process::Child`). Se usan para enviarles señales con
    /// `nix::sys::signal::kill`.
    pty_pids: Arc<Mutex<Vec<u32>>>,
    /// Canal opcional para escribir bytes en el stdin del proceso.
    /// Sólo está cableado en modo PTY; en los otros modos los sends
    /// no llegan a nadie (el receptor se cae al instante).
    stdin_tx: Sender<Vec<u8>>,
    /// PTY master vivo — sólo en modo `Exec::Pty`. Permite resize en
    /// caliente cuando el panel cambia de tamaño. El coordinador del
    /// PTY lo rellena tras crear el `pair` y lo conserva hasta el exit
    /// del child (drop al final).
    pty_master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
}

/// Asa "fría" de un `RunHandle` que **sólo** sirve para matar el comando.
/// No comparte el lock principal de eventos, así que se puede usar desde
/// otro hilo/task aún cuando el dueño del `RunHandle` esté bloqueado en
/// `next_event()`. Cloneable.
#[derive(Clone)]
pub struct Killer {
    children: Arc<Mutex<Vec<Child>>>,
    pty_pids: Arc<Mutex<Vec<u32>>>,
}

impl Killer {
    /// Manda SIGKILL a todas las etapas vivas del comando. No hace nada
    /// si ya terminaron. La señal va a todo el **grupo de procesos** de
    /// cada etapa — con `bash -c "sleep 30"`, esto mata bash *y* el
    /// sleep hijo (mismo pgid; `spawn_shell` arma cada child con
    /// `process_group(0)`).
    pub fn kill(&self) {
        self.signal(nix::sys::signal::Signal::SIGKILL);
        // Fallback: además de la señal, llamamos a `kill()` del Child
        // para que `wait()` cosechée el exit status sin colgarse (el
        // `kill` de std manda SIGKILL al PID directo y no falla si el
        // proceso ya murió).
        if let Ok(mut guard) = self.children.lock() {
            for c in guard.iter_mut() {
                let _ = c.kill();
            }
        }
    }

    /// PIDs de las etapas que aún consider vivas el coordinador. Puede
    /// estar vacío durante una micro-ventana entre `run()` y el spawn
    /// real — no es un bug, sólo refleja la realidad del scheduling.
    pub fn pids(&self) -> Vec<u32> {
        let mut out = Vec::new();
        if let Ok(g) = self.children.lock() {
            out.extend(g.iter().map(|c| c.id()));
        }
        if let Ok(g) = self.pty_pids.lock() {
            out.extend(g.iter().copied());
        }
        out
    }

    /// SIGTERM — el "kill educado" (Ctrl-C estándar). El proceso suele
    /// limpiar antes de morir. Devuelve `true` si llegó a al menos una
    /// etapa viva.
    pub fn term(&self) -> bool {
        self.signal(nix::sys::signal::Signal::SIGTERM)
    }

    /// SIGSTOP — el proceso pasa a estado "stopped"; no consume CPU y
    /// no produce salida hasta recibir SIGCONT. Útil para "pausar" un
    /// `tail -f` o un build ruidoso sin perderlo.
    pub fn stop(&self) -> bool {
        self.signal(nix::sys::signal::Signal::SIGSTOP)
    }

    /// SIGCONT — reanuda un proceso parado con [`Killer::stop`].
    pub fn cont(&self) -> bool {
        self.signal(nix::sys::signal::Signal::SIGCONT)
    }

    fn signal(&self, sig: nix::sys::signal::Signal) -> bool {
        let pids = self.pids();
        let mut delivered = false;
        for pid in pids {
            let target = nix::unistd::Pid::from_raw(pid as i32);
            // `killpg` busca el grupo cuyo pgid coincide con `pid`:
            // como cada child se lanzó con `process_group(0)`, el child
            // es líder del grupo y `pgid == pid`. Matar el grupo abarca
            // cualquier proceso que el child hubiese forkado.
            if nix::sys::signal::killpg(target, sig).is_ok() {
                delivered = true;
            } else if nix::sys::signal::kill(target, sig).is_ok() {
                // Fallback: si el child no es líder de grupo (PTY usa
                // `portable-pty`, que no garantiza `process_group(0)`),
                // mandamos al PID directo.
                delivered = true;
            }
        }
        delivered
    }
}

impl RunHandle {
    /// Mata todos los procesos del comando. No hace nada si ya terminaron.
    pub fn kill(&self) {
        if let Ok(mut guard) = self.children.lock() {
            for c in guard.iter_mut() {
                let _ = c.kill();
            }
        }
    }

    /// Asa cloneable que sólo permite matar el comando — útil para usar
    /// `kill()` desde otra tarea sin tocar el lock que tiene el reader.
    pub fn killer(&self) -> Killer {
        Killer {
            children: Arc::clone(&self.children),
            pty_pids: Arc::clone(&self.pty_pids),
        }
    }

    /// Escribe bytes en el stdin del proceso. Sólo tiene efecto bajo
    /// [`Exec::Pty`] — el modo TUI cablea un writer thread que recibe
    /// estos bytes y los reenvía al PTY master. En los otros modos, el
    /// send se descarta (no hay listener) y devuelve `false`.
    pub fn write_input(&self, bytes: Vec<u8>) -> bool {
        self.stdin_tx.send(bytes).is_ok()
    }

    /// Reescala el PTY. Sólo aplica bajo [`Exec::Pty`]: lockea el
    /// master vivo y llama a `MasterPty::resize`. Devuelve `false`
    /// silenciosamente si no hay PTY (modos Shell/Direct), el master
    /// no se publicó todavía, o el SO devuelve error.
    pub fn resize(&self, rows: u16, cols: u16) -> bool {
        let Ok(mut guard) = self.pty_master.lock() else {
            return false;
        };
        let Some(master) = guard.as_mut() else {
            return false;
        };
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .is_ok()
    }

    /// Próximo evento, bloqueando hasta que llegue. `None` cuando el
    /// proceso terminó (ya se emitió `Exited`/`Failed`) o el canal se
    /// cerró. Pensado para puentes sync→async (el daemon lo usa para
    /// re-emitir cada evento como un frame del protocolo).
    pub fn next_event(&mut self) -> Option<RunEvent> {
        if self.finished {
            return None;
        }
        match self.rx.recv() {
            Ok(ev) => {
                if ev.is_terminal() {
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

    /// Drena todos los eventos disponibles ahora mismo, sin bloquear.
    pub fn try_events(&mut self) -> Vec<RunEvent> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(ev) => {
                    if ev.is_terminal() {
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

    /// Bloquea hasta que el proceso termine y devuelve todos sus eventos.
    pub fn wait_all(&mut self) -> Vec<RunEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.recv() {
            let terminal = ev.is_terminal();
            out.push(ev);
            if terminal {
                self.finished = true;
            }
        }
        self.finished = true;
        out
    }

    /// `true` si ya se observó el evento terminal.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Asa "fría" para controlar el PTY (stdin + resize) desde otra
    /// tarea/hilo sin tocar el lock de eventos — espejo de [`Killer`]
    /// para el lado de entrada. La usa el daemon: el `RunHandle` se mueve
    /// a un hilo-puente que bloquea en `next_event()`, mientras la tarea
    /// async conserva este `PtyControl` para reenviar las teclas y los
    /// resize que llegan del cliente remoto.
    pub fn pty_control(&self) -> PtyControl {
        PtyControl {
            stdin_tx: self.stdin_tx.clone(),
            pty_master: Arc::clone(&self.pty_master),
        }
    }
}

/// Asa cloneable de **control de entrada** de un run PTY: escribe stdin y
/// reescala, sin compartir el lock de eventos del [`RunHandle`]. Igual que
/// con [`RunHandle::write_input`]/[`RunHandle::resize`], las operaciones
/// son no-op fuera de modo [`Exec::Pty`].
#[derive(Clone)]
pub struct PtyControl {
    stdin_tx: Sender<Vec<u8>>,
    pty_master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
}

impl PtyControl {
    /// Escribe bytes en el stdin del PTY. Ver [`RunHandle::write_input`].
    pub fn write_input(&self, bytes: Vec<u8>) -> bool {
        self.stdin_tx.send(bytes).is_ok()
    }

    /// Reescala el PTY. Ver [`RunHandle::resize`].
    pub fn resize(&self, rows: u16, cols: u16) -> bool {
        let Ok(mut guard) = self.pty_master.lock() else {
            return false;
        };
        let Some(master) = guard.as_mut() else {
            return false;
        };
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .is_ok()
    }
}

/// Vuelca el resto de un pipe a un archivo con **copia cero** (`splice`):
/// los bytes van de pipe a archivo sin pasar por espacio de usuario.
fn spill_rest<R: Read + AsFd>(reader: &mut BufReader<R>, path: &Path, first_line: &str) {
    let Ok(file) = File::create(path) else {
        return;
    };
    let mut file = file;
    // La línea que cruzó el tope y lo ya bufereado van primero…
    let _ = file.write_all(first_line.as_bytes());
    let buffered: Vec<u8> = reader.buffer().to_vec();
    let _ = file.write_all(&buffered);
    reader.consume(buffered.len());
    // …y el resto del pipe se mueve con `splice`, kernel a kernel.
    loop {
        match splice(reader.get_ref(), None, &file, None, 1 << 20, SpliceFFlags::empty()) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

/// Lanza un hilo lector de un flujo, con captura acotada. Pasado el tope:
/// si hay `spill`, el resto se vuelca al archivo con `splice` (copia
/// cero); si no, se descarta. En ambos casos el pipe se **sigue
/// drenando** — el proceso nunca se bloquea.
#[allow(clippy::too_many_arguments)]
fn spawn_reader<R: Read + AsFd + Send + 'static>(
    stream: R,
    tx: Sender<RunEvent>,
    make: fn(String) -> RunEvent,
    limit: usize,
    counter: Arc<AtomicUsize>,
    announced: Arc<AtomicBool>,
    spill: Option<PathBuf>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = match read_line_loose(&mut reader, &mut buf) {
                Ok(0) => break, // EOF
                Ok(n) => n,
                Err(_) => break,
            };
            let total = counter.fetch_add(n, Ordering::Relaxed) + n;
            if limit != 0 && total > limit {
                let first = !announced.swap(true, Ordering::Relaxed);
                match &spill {
                    Some(path) => {
                        if first {
                            let _ = tx.send(RunEvent::Spilled(path.display().to_string()));
                        }
                        spill_rest(&mut reader, path, &buf);
                        break; // splice se llevó el resto
                    }
                    None => {
                        if first {
                            let _ = tx.send(RunEvent::Truncated);
                        }
                        continue; // descarta, pero sigue drenando
                    }
                }
            }
            let line = buf.trim_end_matches(['\n', '\r']).to_string();
            if tx.send(make(line)).is_err() {
                break;
            }
        }
    })
}

/// Como `BufRead::read_line`, pero corta también en `\r` (no solo `\n`).
/// Pensado para que **progress bars** estilo `wget`/`pip install -v`/`curl`
/// se muestren en vivo: esos escriben `\r` para sobreescribir la misma
/// línea y nunca emiten `\n` hasta el final — con `read_line` clásico el
/// usuario no ve nada hasta que el comando termina.
///
/// El `\n` posterior a un `\r` (`\r\n` clásico de Windows o de `git log`
/// pasado por less) se ve como una línea vacía adicional — aceptable a
/// cambio de tener feedback en vivo en TUIs no-PTY.
fn read_line_loose<R: BufRead>(reader: &mut R, buf: &mut String) -> std::io::Result<usize> {
    let mut bytes: Vec<u8> = Vec::with_capacity(128);
    let mut total = 0;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break; // EOF
        }
        if let Some(pos) = chunk.iter().position(|&b| b == b'\n' || b == b'\r') {
            bytes.extend_from_slice(&chunk[..=pos]);
            let consumed = pos + 1;
            reader.consume(consumed);
            total += consumed;
            break;
        } else {
            bytes.extend_from_slice(chunk);
            let n = chunk.len();
            reader.consume(n);
            total += n;
        }
    }
    if !bytes.is_empty() {
        buf.push_str(&String::from_utf8_lossy(&bytes));
    }
    Ok(total)
}

/// Resultado de lanzar los procesos: lo que el coordinador necesita.
struct Spawned {
    children: Vec<Child>,
    stdin: Option<std::process::ChildStdin>,
    stdout: Option<std::process::ChildStdout>,
    stderrs: Vec<std::process::ChildStderr>,
    /// Etapas intermedias a interceptar (solo con `capture_stages`). Vacío
    /// en el caso normal.
    stage_tees: Vec<StageTee>,
}

/// Una etapa intermedia cuyo stdout interceptamos: el coordinador lee
/// `stdout`, reenvía los bytes a `sink` (el stdin de la etapa siguiente) y
/// emite cada línea como [`RunEvent::StageStdout`].
struct StageTee {
    stage: usize,
    stdout: std::process::ChildStdout,
    sink: std::fs::File,
}

/// Lanza un único proceso shell (`program -c "<line>"`). `_want_stdin` se
/// mantiene por compatibilidad de firma: ahora stdin SIEMPRE se abre como
/// `piped` para que el usuario pueda alimentar Y/n a prompts interactivos
/// (apt, pacman, sudo, etc.). Los comandos que no leen stdin no se
/// afectan; los que sí (cat sin args, head -) se cuelgan esperando — lo
/// cual es el comportamiento esperado de un shell real.
fn spawn_shell(line: &str, program: &str, cwd: &str, _want_stdin: bool) -> std::io::Result<Spawned> {
    let mut child = Command::new(program)
        .arg("-c")
        .arg(line)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Nuevo grupo de procesos: con `bash -c "sleep 30"` el bash se
        // forka a un sleep hijo; matar al bash sólo no alcanza al sleep.
        // Con el grupo, `killpg(pid, SIG)` derriba a todo el subárbol.
        .process_group(0)
        .spawn()?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderrs = child.stderr.take().into_iter().collect();
    Ok(Spawned { children: vec![child], stdin, stdout, stderrs, stage_tees: vec![] })
}

/// Lanza un pipe de etapas conectándolas con descriptores reales.
fn spawn_direct(
    stages: &[StageSpec],
    cwd: &str,
    want_stdin: bool,
    capture_stages: bool,
) -> std::io::Result<Spawned> {
    if stages.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "pipe vacío",
        ));
    }
    let n = stages.len();
    let mut children: Vec<Child> = Vec::with_capacity(n);
    let mut stage_tees: Vec<StageTee> = Vec::new();
    // Qué alimenta el stdin de la etapa actual (i>0): el stdout de la
    // anterior (directo) o el read-end de un pipe de tee (capturado).
    let mut next_stdin: Option<Stdio> = None;

    for (i, st) in stages.iter().enumerate() {
        let mut cmd = Command::new(&st.program);
        cmd.args(&st.args)
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if i == 0 {
            cmd.stdin(if want_stdin { Stdio::piped() } else { Stdio::null() });
            // Primera etapa abre su propio grupo de procesos; las demás
            // se enganchan en el mismo (process_group(0) hereda el pgid
            // del padre — pero el padre somos nosotros, no la etapa 0).
            // En la práctica `setpgid` del kernel respeta sólo lo que
            // pedimos: las etapas 1.. quedan en pgid de la 0 si las
            // forkamos con CLONE_PARENT_SETTID; con Command no tenemos
            // ese control fino, así que cada stage queda en su propio
            // pgid. El Killer cubre todas las etapas igual porque
            // mantiene los PIDs/Childs por separado.
            cmd.process_group(0);
        } else {
            // La etapa anterior alimenta a ésta (stdout directo o tee).
            cmd.stdin(next_stdin.take().expect("stdin de etapa previa"));
            cmd.process_group(0);
        }
        match cmd.spawn() {
            Ok(mut child) => {
                if i + 1 < n {
                    let stdout = child.stdout.take().expect("stdout de etapa intermedia");
                    if capture_stages {
                        // Tee: pipe propio. La etapa siguiente lee del read-end;
                        // un hilo del coordinador reenvía stage[i].stdout al
                        // write-end y captura cada línea como StageStdout.
                        // `O_CLOEXEC`: estos fds NO deben heredarse a las etapas
                        // que spawneamos después. Si una etapa heredara el
                        // write-end del pipe de tee, su lado lector nunca vería
                        // EOF y se colgaría esperando más entrada (deadlock). El
                        // dup2 a fd 0 de la etapa siguiente lo hace std (limpia
                        // CLOEXEC en el fd 0 resultante), así que su stdin queda bien.
                        let (rd, wr) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)
                            .map_err(|e| std::io::Error::other(format!("pipe tee: {e}")))?;
                        next_stdin = Some(Stdio::from(std::fs::File::from(rd)));
                        stage_tees.push(StageTee {
                            stage: i,
                            stdout,
                            sink: std::fs::File::from(wr),
                        });
                    } else {
                        next_stdin = Some(Stdio::from(stdout));
                    }
                }
                children.push(child);
            }
            Err(e) => {
                // Si una etapa no arranca, se matan las ya lanzadas.
                for mut c in children {
                    let _ = c.kill();
                }
                return Err(std::io::Error::new(
                    e.kind(),
                    format!("{}: {e}", st.program),
                ));
            }
        }
    }

    let stdin = children.first_mut().and_then(|c| c.stdin.take());
    let stdout = children.last_mut().and_then(|c| c.stdout.take());
    let stderrs = children.iter_mut().filter_map(|c| c.stderr.take()).collect();
    Ok(Spawned { children, stdin, stdout, stderrs, stage_tees })
}

/// Hilo de tee de una etapa intermedia: lee su stdout, lo reenvía a `sink`
/// (stdin de la etapa siguiente) y emite cada línea como `StageStdout`. Al
/// EOF cierra `sink` (drop) para que la etapa siguiente vea fin de entrada.
fn tee_pump(
    stage: usize,
    mut stdout: std::process::ChildStdout,
    mut sink: std::fs::File,
    tx: Sender<RunEvent>,
) {
    let mut buf = [0u8; 8192];
    let mut line: Vec<u8> = Vec::new();
    loop {
        let n = match stdout.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let chunk = &buf[..n];
        // Reenviar a la etapa siguiente (si murió, igual seguimos drenando
        // para no bloquear a la etapa actual).
        let _ = sink.write_all(chunk);
        // Capturar por línea para el desplegable de la etapa.
        for &b in chunk {
            if b == b'\n' {
                let s = String::from_utf8_lossy(&line).into_owned();
                let _ = tx.send(RunEvent::StageStdout { stage, line: s });
                line.clear();
            } else {
                line.push(b);
            }
        }
    }
    if !line.is_empty() {
        let s = String::from_utf8_lossy(&line).into_owned();
        let _ = tx.send(RunEvent::StageStdout { stage, line: s });
    }
}

/// Lanza `spec` y devuelve un [`RunHandle`] desde el que drenar la
/// salida. La función vuelve de inmediato: el proceso corre en hilos.
pub fn run(spec: &CommandSpec) -> RunHandle {
    let (tx, rx) = mpsc::channel();
    let (stdin_tx, stdin_rx) = mpsc::channel::<Vec<u8>>();
    let spec = spec.clone();
    let cell: Arc<Mutex<Vec<Child>>> = Arc::new(Mutex::new(Vec::new()));
    let pty_pids: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
    let pty_master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>> =
        Arc::new(Mutex::new(None));
    let cell_thread = Arc::clone(&cell);
    let pty_pids_thread = Arc::clone(&pty_pids);
    let pty_master_thread = Arc::clone(&pty_master);

    // Modo PTY: ruta separada — el proceso corre bajo un pseudo-terminal
    // (cross-platform via `portable-pty`), los bytes crudos se emiten
    // como [`RunEvent::Bytes`] para que el frontend los pase por su
    // emulador vt100, y el frontend escribe en stdin con
    // [`RunHandle::write_input`].
    if let Exec::Pty { program, args, cols, rows } = &spec.exec {
        let program = program.clone();
        let args = args.clone();
        let cols = *cols;
        let rows = *rows;
        let cwd = spec.cwd.clone();
        std::thread::spawn(move || {
            spawn_pty_thread(
                &program,
                &args,
                &cwd,
                cols,
                rows,
                tx,
                stdin_rx,
                pty_pids_thread,
                pty_master_thread,
            );
        });
        return RunHandle {
            rx,
            finished: false,
            children: cell,
            pty_pids,
            stdin_tx,
            pty_master,
        };
    }

    std::thread::spawn(move || {
        let want_stdin = spec.stdin_data.is_some();
        let spawned = match &spec.exec {
            Exec::Shell { line, program } => {
                spawn_shell(line, program, &spec.cwd, want_stdin)
            }
            Exec::Direct { stages } => {
                spawn_direct(stages, &spec.cwd, want_stdin, spec.capture_stages)
            }
            Exec::Pty { .. } => unreachable!("Pty se maneja antes"),
        };
        let Spawned { children, stdin, stdout, stderrs, stage_tees } = match spawned {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(RunEvent::Failed(e.to_string()));
                return;
            }
        };

        // Alimenta stdin. Hay DOS modos según si el caller dio `stdin_data`:
        //
        // - **Reprocess** (`stdin_data = Some(...)`): escribimos los bytes
        //   y CERRAMOS el sink inmediatamente. El child recibe EOF y procesa
        //   normal (sort, head, jq…). El input vivo NO aplica acá.
        //
        // - **Interactivo** (`stdin_data = None`): el thread queda leyendo
        //   `stdin_rx` para que el usuario pueda responder prompts (apt Y/n,
        //   sudo password, etc.) tipeando en el input box. Sale cuando el
        //   channel cierra (`RunHandle` droppeado) o el child cierra stdin.
        if let Some(mut sink) = stdin {
            let initial = spec.stdin_data.clone();
            std::thread::spawn(move || {
                if let Some(data) = initial {
                    let _ = sink.write_all(data.as_bytes());
                    // Drop del sink al salir = EOF para el child.
                    return;
                }
                while let Ok(bytes) = stdin_rx.recv() {
                    if sink.write_all(&bytes).is_err() {
                        break;
                    }
                    let _ = sink.flush();
                }
            });
        }

        // Comparte los procesos para que `kill` los alcance.
        if let Ok(mut g) = cell_thread.lock() {
            *g = children;
        }

        // Tee de etapas intermedias (solo con capture_stages): un hilo por
        // etapa que reenvía su stdout a la siguiente y emite StageStdout.
        // Guardamos los handles para joinearlos antes de `Exited` (si no, un
        // StageStdout tardío se perdería tras cerrar el run).
        let mut tee_handles: Vec<JoinHandle<()>> = Vec::new();
        for tee in stage_tees {
            let txc = tx.clone();
            tee_handles
                .push(std::thread::spawn(move || tee_pump(tee.stage, tee.stdout, tee.sink, txc)));
        }

        // Captura acotada: contador y aviso compartidos por todos los
        // lectores. El volcado a archivo se aplica sólo a stdout (el
        // contenido principal); stderr excedente se descarta.
        let counter = Arc::new(AtomicUsize::new(0));
        let announced = Arc::new(AtomicBool::new(false));
        let limit = spec.capture_limit;

        let mut readers: Vec<JoinHandle<()>> = Vec::new();
        if let Some(s) = stdout {
            readers.push(spawn_reader(
                s,
                tx.clone(),
                RunEvent::Stdout,
                limit,
                Arc::clone(&counter),
                Arc::clone(&announced),
                spec.spill_path.clone(),
            ));
        }
        for s in stderrs {
            readers.push(spawn_reader(
                s,
                tx.clone(),
                RunEvent::Stderr,
                limit,
                Arc::clone(&counter),
                Arc::clone(&announced),
                None,
            ));
        }
        for h in readers {
            let _ = h.join();
        }
        // Joinear los tees garantiza que todos los StageStdout salgan antes
        // del Exited (drenaje completo de las etapas intermedias).
        for h in tee_handles {
            let _ = h.join();
        }

        // Cosecha todas las etapas; el código de salida es el de la última.
        let code = {
            let mut g = cell_thread.lock().expect("children lock");
            let mut last = -1;
            for c in g.iter_mut() {
                last = c.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
            }
            last
        };
        let _ = tx.send(RunEvent::Exited(code));
    });

    RunHandle {
        rx,
        finished: false,
        children: cell,
        pty_pids,
        stdin_tx,
        pty_master,
    }
}

/// Coordinador del modo PTY: aloja un PTY de tamaño `cols`×`rows`,
/// lanza el comando bajo él, y mantiene tres flujos:
///
/// - Reader thread: lee chunks de hasta 4 KiB del master del PTY y los
///   emite como `RunEvent::Bytes`. Termina cuando el child cierra el
///   slave (EOF).
/// - Writer thread: bloquea en `stdin_rx` y reenvía bytes al master
///   del PTY (lo que el frontend manda como input crudo).
/// - Esta función espera al child y emite `RunEvent::Exited(code)`.
fn spawn_pty_thread(
    program: &str,
    args: &[String],
    cwd: &str,
    cols: u16,
    rows: u16,
    tx: Sender<RunEvent>,
    stdin_rx: Receiver<Vec<u8>>,
    pty_pids: Arc<Mutex<Vec<u32>>>,
    pty_master_slot: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
) {
    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(p) => p,
        Err(e) => {
            let _ = tx.send(RunEvent::Failed(format!("openpty: {e}")));
            return;
        }
    };
    let mut cmd = CommandBuilder::new(program);
    for a in args {
        cmd.arg(a);
    }
    cmd.cwd(cwd);
    // `CommandBuilder` de portable_pty arranca con env vacío — hay que
    // heredar manualmente PATH/HOME/USER/SUDO_ASKPASS/SSH_ASKPASS/etc.
    // Sin esto, `sudo -A` no encuentra el askpass y `which` falla.
    for (k, v) in std::env::vars_os() {
        cmd.env(k, v);
    }
    // Heurística estándar: TUIs leen `TERM` para decidir capacidad de
    // colores y movimiento. xterm-256color es el lcm más amplio. Se
    // sobreescribe el TERM heredado por si el caller corre desde una
    // shell sin TERM (cron, systemd-run, etc.).
    cmd.env("TERM", "xterm-256color");
    let mut child = match pair.slave.spawn_command(cmd) {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(RunEvent::Failed(format!("spawn: {e}")));
            return;
        }
    };
    // El slave fd no se necesita más en el padre; cerrarlo aquí evita
    // que el child quede sin EOF cuando cierra su lado.
    drop(pair.slave);

    if let Some(pid) = child.process_id() {
        if let Ok(mut g) = pty_pids.lock() {
            g.push(pid);
        }
    }

    let mut reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(RunEvent::Failed(format!("try_clone_reader: {e}")));
            return;
        }
    };
    let mut writer = match pair.master.take_writer() {
        Ok(w) => w,
        Err(e) => {
            let _ = tx.send(RunEvent::Failed(format!("take_writer: {e}")));
            return;
        }
    };

    // Publica el master vivo: el `RunHandle` lo lockea para resize en
    // caliente. `take_writer`/`try_clone_reader` ya retornaron handles
    // independientes; el master se mantiene como dueño del PTY.
    if let Ok(mut slot) = pty_master_slot.lock() {
        *slot = Some(pair.master);
    }

    let tx_reader = tx.clone();
    let reader_thread = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // child cerró el slave
                Ok(n) => {
                    if tx_reader.send(RunEvent::Bytes(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let writer_thread = std::thread::spawn(move || {
        while let Ok(bytes) = stdin_rx.recv() {
            if writer.write_all(&bytes).is_err() {
                break;
            }
            let _ = writer.flush();
        }
    });

    let code = match child.wait() {
        Ok(s) => s.exit_code() as i32,
        Err(_) => -1,
    };
    // El master se dropea ahora → reader ve EOF y termina. Esperamos al
    // reader para que no se pierdan bytes finales.
    if let Ok(mut slot) = pty_master_slot.lock() {
        *slot = None;
    }
    let _ = reader_thread.join();
    // El writer thread sale solo cuando el stdin_tx se dropee del lado
    // del frontend; no lo joineamos sincrónicamente.
    drop(writer_thread);
    let _ = tx.send(RunEvent::Exited(code));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ejecución directa de un único programa.
    fn direct(program: &str, args: &[&str]) -> CommandSpec {
        CommandSpec::direct(
            vec![StageSpec {
                program: program.into(),
                args: args.iter().map(|s| s.to_string()).collect(),
            }],
            ".",
        )
    }

    /// Pipe directo de varias etapas.
    fn pipe(stages: &[(&str, &[&str])]) -> CommandSpec {
        CommandSpec::direct(
            stages
                .iter()
                .map(|(p, a)| StageSpec {
                    program: p.to_string(),
                    args: a.iter().map(|s| s.to_string()).collect(),
                })
                .collect(),
            ".",
        )
    }

    fn stdout_of(events: Vec<RunEvent>) -> Vec<String> {
        events
            .into_iter()
            .filter_map(|e| match e {
                RunEvent::Stdout(l) => Some(l),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn direct_runs_a_single_program() {
        let mut h = run(&direct("echo", &["hola", "mundo"]));
        let events = h.wait_all();
        assert!(events.contains(&RunEvent::Stdout("hola mundo".into())));
        assert!(events.contains(&RunEvent::Exited(0)));
    }

    #[test]
    fn direct_wires_a_pipeline_itself() {
        // printf … | sort — brahman conecta los procesos, sin shell.
        let mut h = run(&pipe(&[
            ("printf", &["b\\na\\nc\\n"]),
            ("sort", &[]),
        ]));
        assert_eq!(stdout_of(h.wait_all()), vec!["a", "b", "c"]);
    }

    #[test]
    fn direct_three_stage_pipeline() {
        let mut h = run(&pipe(&[
            ("printf", &["3\\n1\\n2\\n1\\n"]),
            ("sort", &[]),
            ("uniq", &[]),
        ]));
        assert_eq!(stdout_of(h.wait_all()), vec!["1", "2", "3"]);
    }

    fn stage_lines(events: &[RunEvent], stage: usize) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                RunEvent::StageStdout { stage: s, line } if *s == stage => Some(line.clone()),
                _ => None,
            })
            .collect()
    }

    // El supuesto "deadlock por write-end huérfano" era en realidad el error de
    // build: se pidió la feature `fcntl` de nix (inexistente en 0.29 — `OFlag`
    // vive bajo `fs`, ya habilitada), así que el crate ni compilaba y el fallo
    // se confundió con un cuelgue. Con pipe2(O_CLOEXEC) ningún hijo hereda el
    // write-end del tee y la etapa siguiente ve EOF en cuanto la anterior cierra
    // su stdout; verificado estable (5/5).
    #[test]
    fn capture_stages_intercepts_each_stage_stdout() {
        // printf "hello" | tr a-z A-Z | rev  →  etapa0="hello", etapa1="HELLO",
        // salida final (rev) = "OLLEH". El tee captura las intermedias EN VIVO
        // sin re-ejecutar, que es lo que el desplegable necesita por etapa.
        let spec = pipe(&[
            ("printf", &["hello\\n"]),
            ("tr", &["a-z", "A-Z"]),
            ("rev", &[]),
        ])
        .with_stage_capture();
        let events = run(&spec).wait_all();

        assert_eq!(stage_lines(&events, 0), vec!["hello"], "etapa 0 (printf)");
        assert_eq!(stage_lines(&events, 1), vec!["HELLO"], "etapa 1 (tr)");
        // La última etapa sigue saliendo por Stdout normal.
        assert_eq!(stdout_of(events), vec!["OLLEH"]);
    }

    #[test]
    fn without_capture_stages_there_are_no_stage_events() {
        // El comportamiento por defecto no cambia: sólo la salida final.
        let events = run(&pipe(&[("printf", &["x\\n"]), ("cat", &[])])).wait_all();
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, RunEvent::StageStdout { .. })),
            "sin with_stage_capture no debe haber StageStdout"
        );
        assert_eq!(stdout_of(events), vec!["x"]);
    }

    #[test]
    fn direct_nonzero_exit_is_the_last_stage() {
        let mut h = run(&direct("false", &[]));
        assert!(h.wait_all().contains(&RunEvent::Exited(1)));
    }

    #[test]
    fn direct_missing_program_fails_gracefully() {
        let mut h = run(&direct("no-existe-binario-xyz", &[]));
        let events = h.wait_all();
        assert!(matches!(events.first(), Some(RunEvent::Failed(_))));
    }

    #[test]
    fn shell_mode_still_works_for_complex_syntax() {
        let mut h = run(&CommandSpec {
            exec: Exec::Shell { line: "echo $((2 + 3))".into(), program: "sh".into() },
            ..CommandSpec::shell("", ".")
        });
        assert!(h.wait_all().contains(&RunEvent::Stdout("5".into())));
    }

    #[test]
    fn capture_limit_truncates_but_process_finishes() {
        let mut h = run(&direct("seq", &["1", "20000"]).with_limit(400));
        let events = h.wait_all();
        assert!(events.contains(&RunEvent::Truncated));
        assert!(events.contains(&RunEvent::Exited(0)));
        assert!(stdout_of(events).len() < 20000);
    }

    #[test]
    fn spill_writes_overflow_to_a_file() {
        let path = std::env::temp_dir()
            .join(format!("shuma-exec-spill-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut h = run(&direct("seq", &["1", "5000"])
            .with_limit(200)
            .with_spill(path.clone()));
        let events = h.wait_all();
        assert!(events.iter().any(|e| matches!(e, RunEvent::Spilled(_))));
        assert!(events.contains(&RunEvent::Exited(0)));
        // El archivo de volcado existe y tiene contenido.
        let spilled = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(spilled.contains("5000"), "la cola se volcó al archivo");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stdin_data_reprocessed_by_a_filter() {
        let mut h = run(&direct("grep", &["beta"]).with_stdin("alfa\nbeta\nbetabel\ngamma"));
        assert_eq!(stdout_of(h.wait_all()), vec!["beta", "betabel"]);
    }

    #[test]
    fn kill_stops_a_long_running_pipeline() {
        let mut h = run(&pipe(&[("sleep", &["30"]), ("cat", &[])]));
        std::thread::sleep(std::time::Duration::from_millis(250));
        h.kill();
        let events = h.wait_all();
        assert!(events.last().map(|e| e.is_terminal()).unwrap_or(false));
    }

    #[test]
    fn terminal_event_detection() {
        assert!(RunEvent::Exited(0).is_terminal());
        assert!(!RunEvent::Truncated.is_terminal());
        assert!(!RunEvent::Spilled("x".into()).is_terminal());
    }

    #[test]
    fn pty_run_emits_bytes_for_a_tty_aware_command() {
        // `tty` imprime el path del terminal cuando se le da uno, o
        // "not a tty" cuando se le pipea. Bajo PTY debe imprimir un
        // path con `/dev/pts/` o `/dev/ptmx` (depende del SO).
        let spec = CommandSpec {
            exec: Exec::Pty {
                program: "tty".into(),
                args: vec![],
                cols: 80,
                rows: 24,
            },
            cwd: ".".into(),
            capture_limit: 0,
            spill_path: None,
            stdin_data: None,
            capture_stages: false,
        };
        let mut h = run(&spec);
        let mut bytes_seen = Vec::<u8>::new();
        let mut exit = -999;
        while let Some(ev) = h.next_event() {
            match ev {
                RunEvent::Bytes(b) => bytes_seen.extend(b),
                RunEvent::Exited(c) => exit = c,
                _ => {}
            }
        }
        assert_eq!(exit, 0, "tty exit 0 bajo PTY (bytes='{}')",
                   String::from_utf8_lossy(&bytes_seen));
        let out = String::from_utf8_lossy(&bytes_seen);
        // `tty` emite el path del slave + \r\n (los PTY añaden \r).
        assert!(
            out.contains("/dev/pts/") || out.contains("/dev/ptmx") || out.contains("/dev/tty"),
            "tty output inesperado: {out:?}"
        );
    }

    #[test]
    fn pty_write_input_reaches_the_child_stdin() {
        // `cat` bajo PTY refleja lo que le escribamos por stdin (en
        // modo PTY, cada caracter va con echo encendido por defecto).
        // Le mandamos "hola\n" y esperamos verlo en los bytes de
        // salida. Después cerramos el stdin (drop del Sender) y
        // mandamos Ctrl-D (0x04) para que cat termine.
        let spec = CommandSpec {
            exec: Exec::Pty {
                program: "cat".into(),
                args: vec![],
                cols: 80,
                rows: 24,
            },
            cwd: ".".into(),
            capture_limit: 0,
            spill_path: None,
            stdin_data: None,
            capture_stages: false,
        };
        let mut h = run(&spec);
        // Cat necesita un instante para que su slave arranque y abra stdin.
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(h.write_input(b"hola\n".to_vec()));
        // Ctrl-D para cerrar EOF en modo cooked.
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(h.write_input(b"\x04".to_vec()));
        let mut bytes_seen = Vec::<u8>::new();
        let started = std::time::Instant::now();
        while let Some(ev) = h.next_event() {
            if started.elapsed() > std::time::Duration::from_secs(5) {
                panic!("cat no terminó en 5s tras Ctrl-D");
            }
            if let RunEvent::Bytes(b) = ev {
                bytes_seen.extend(b);
            }
        }
        let out = String::from_utf8_lossy(&bytes_seen);
        assert!(out.contains("hola"), "cat no echó 'hola': {out:?}");
    }

    #[test]
    fn pty_resize_changes_dimensions_in_running_child() {
        // Lanzamos `bash` bajo PTY a 80×24, esperamos que el slave
        // tenga las dims iniciales, hacemos resize, y verificamos que
        // el child ve el nuevo tamaño tras la señal SIGWINCH.
        let spec = CommandSpec {
            exec: Exec::Pty {
                program: "bash".into(),
                args: vec!["-c".into(), "stty size; sleep 0.3; stty size".into()],
                cols: 80,
                rows: 24,
            },
            cwd: ".".into(),
            capture_limit: 0,
            spill_path: None,
            stdin_data: None,
            capture_stages: false,
        };
        let mut h = run(&spec);
        // Esperar a que el master se publique (race con el coordinador).
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(h.resize(40, 132), "resize debería aplicarse");
        let mut bytes = Vec::<u8>::new();
        while let Some(ev) = h.next_event() {
            if let RunEvent::Bytes(b) = ev {
                bytes.extend(b);
            }
        }
        let out = String::from_utf8_lossy(&bytes);
        // El primer `stty size` muestra 24 80; el segundo debe mostrar
        // las dimensiones nuevas tras el resize (40 132).
        assert!(
            out.contains("40 132"),
            "stty size tras resize no muestra 40 132: {out:?}"
        );
    }

    #[test]
    fn killer_can_stop_and_continue_a_process() {
        // `sleep 30` se para con SIGSTOP, se reanuda con SIGCONT y
        // luego se mata con SIGTERM. El test acaba en <1s aunque el
        // sleep nominal sea de 30s.
        let h = run(&direct("sleep", &["30"]));
        let killer = h.killer();
        // Esperar a que aparezca el PID (el coordinador rellena el Vec
        // tras el spawn — micro-delay).
        let mut tries = 0;
        while killer.pids().is_empty() && tries < 100 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            tries += 1;
        }
        assert!(!killer.pids().is_empty(), "pid no apareció");
        assert!(killer.stop(), "SIGSTOP no llegó");
        assert!(killer.cont(), "SIGCONT no llegó");
        assert!(killer.term(), "SIGTERM no llegó");
        // El test no se cuelga gracias al term().
    }
}
