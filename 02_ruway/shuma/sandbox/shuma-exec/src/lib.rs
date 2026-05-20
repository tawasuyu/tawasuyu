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
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

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
        }
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
    /// Una línea de salida estándar.
    Stdout(String),
    /// Una línea de salida de error.
    Stderr(String),
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

/// Destino de volcado de la salida excedente — compartido entre lectores.
struct SpillSink {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

impl SpillSink {
    /// Escribe una línea excedente al archivo (lo abre perezosamente).
    fn write_line(&self, line: &str) {
        let mut g = self.file.lock().expect("spill lock");
        if g.is_none() {
            *g = File::create(&self.path).ok();
        }
        if let Some(f) = g.as_mut() {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Asa de un comando en ejecución. El consumidor la conserva y drena sus
/// eventos cuando le conviene.
pub struct RunHandle {
    rx: Receiver<RunEvent>,
    finished: bool,
    /// Los procesos, compartidos con el hilo coordinador para poder
    /// matarlos — todas las etapas de un pipe directo.
    children: Arc<Mutex<Vec<Child>>>,
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
}

/// Lanza un hilo lector de un flujo, con captura acotada. Pasado el tope
/// emite (una vez) `Truncated` o `Spilled` y deriva el resto al sumidero
/// de volcado o a la basura — pero **sigue drenando** el pipe.
#[allow(clippy::too_many_arguments)]
fn spawn_reader<R: Read + Send + 'static>(
    stream: R,
    tx: Sender<RunEvent>,
    make: fn(String) -> RunEvent,
    limit: usize,
    counter: Arc<AtomicUsize>,
    announced: Arc<AtomicBool>,
    spill: Option<Arc<SpillSink>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            let total =
                counter.fetch_add(line.len() + 1, Ordering::Relaxed) + line.len() + 1;
            if limit != 0 && total > limit {
                let first = !announced.swap(true, Ordering::Relaxed);
                match &spill {
                    Some(sink) => {
                        if first {
                            let _ = tx
                                .send(RunEvent::Spilled(sink.path.display().to_string()));
                        }
                        sink.write_line(&line);
                    }
                    None => {
                        if first {
                            let _ = tx.send(RunEvent::Truncated);
                        }
                    }
                }
                continue; // descarta/vuelca, pero sigue leyendo el pipe
            }
            if tx.send(make(line)).is_err() {
                break;
            }
        }
    })
}

/// Resultado de lanzar los procesos: lo que el coordinador necesita.
struct Spawned {
    children: Vec<Child>,
    stdin: Option<std::process::ChildStdin>,
    stdout: Option<std::process::ChildStdout>,
    stderrs: Vec<std::process::ChildStderr>,
}

/// Lanza un único proceso shell (`program -c "<line>"`).
fn spawn_shell(line: &str, program: &str, cwd: &str, want_stdin: bool) -> std::io::Result<Spawned> {
    let mut child = Command::new(program)
        .arg("-c")
        .arg(line)
        .current_dir(cwd)
        .stdin(if want_stdin { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderrs = child.stderr.take().into_iter().collect();
    Ok(Spawned { children: vec![child], stdin, stdout, stderrs })
}

/// Lanza un pipe de etapas conectándolas con descriptores reales.
fn spawn_direct(stages: &[StageSpec], cwd: &str, want_stdin: bool) -> std::io::Result<Spawned> {
    if stages.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "pipe vacío",
        ));
    }
    let n = stages.len();
    let mut children: Vec<Child> = Vec::with_capacity(n);
    let mut prev_stdout: Option<std::process::ChildStdout> = None;

    for (i, st) in stages.iter().enumerate() {
        let mut cmd = Command::new(&st.program);
        cmd.args(&st.args)
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if i == 0 {
            cmd.stdin(if want_stdin { Stdio::piped() } else { Stdio::null() });
        } else {
            // La etapa anterior alimenta a ésta por su stdout.
            cmd.stdin(Stdio::from(prev_stdout.take().expect("stdout previo")));
        }
        match cmd.spawn() {
            Ok(mut child) => {
                if i + 1 < n {
                    prev_stdout = child.stdout.take();
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
    Ok(Spawned { children, stdin, stdout, stderrs })
}

/// Lanza `spec` y devuelve un [`RunHandle`] desde el que drenar la
/// salida. La función vuelve de inmediato: el proceso corre en hilos.
pub fn run(spec: &CommandSpec) -> RunHandle {
    let (tx, rx) = mpsc::channel();
    let spec = spec.clone();
    let cell: Arc<Mutex<Vec<Child>>> = Arc::new(Mutex::new(Vec::new()));
    let cell_thread = Arc::clone(&cell);

    std::thread::spawn(move || {
        let want_stdin = spec.stdin_data.is_some();
        let spawned = match &spec.exec {
            Exec::Shell { line, program } => {
                spawn_shell(line, program, &spec.cwd, want_stdin)
            }
            Exec::Direct { stages } => spawn_direct(stages, &spec.cwd, want_stdin),
        };
        let Spawned { children, stdin, stdout, stderrs } = match spawned {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(RunEvent::Failed(e.to_string()));
                return;
            }
        };

        // Alimenta stdin (reproceso) en su propio hilo.
        if let (Some(data), Some(mut sink)) = (spec.stdin_data.clone(), stdin) {
            std::thread::spawn(move || {
                let _ = sink.write_all(data.as_bytes());
            });
        }

        // Comparte los procesos para que `kill` los alcance.
        if let Ok(mut g) = cell_thread.lock() {
            *g = children;
        }

        // Captura acotada: contador y aviso compartidos por todos los
        // lectores; un sumidero de volcado opcional.
        let counter = Arc::new(AtomicUsize::new(0));
        let announced = Arc::new(AtomicBool::new(false));
        let spill = spec
            .spill_path
            .clone()
            .map(|path| Arc::new(SpillSink { path, file: Mutex::new(None) }));
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
                spill.clone(),
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
                spill.clone(),
            ));
        }
        for h in readers {
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

    RunHandle { rx, finished: false, children: cell }
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
}
