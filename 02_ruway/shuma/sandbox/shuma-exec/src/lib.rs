//! `shuma-exec` — ejecución de comandos del shell con salida en streaming.
//!
//! Lanza una línea de comandos en un shell (`bash -c …`) dentro de un
//! directorio, y entrega su salida **a medida que ocurre**: cada línea
//! de stdout o stderr llega como un [`RunEvent`] por un canal, sin
//! esperar a que el proceso termine.
//!
//! Esto es lo que `sandokan` no hace: el orquestador es poll-based y
//! orquesta *Cards* de brahman (entidades aisladas y supervisadas). El
//! shell, en cambio, corre líneas de shell ad-hoc y necesita ver la
//! salida fluir. Dos capas distintas, a propósito.
//!
//! **Captura acotada.** Para no cargar en RAM un stream de gigabytes, la
//! captura tiene un límite de bytes ([`CommandSpec::capture_limit`]):
//! pasado el límite se emite [`RunEvent::Truncated`] una vez y el resto
//! se **descarta** — pero el pipe se sigue drenando, así el proceso no
//! se bloquea y termina normal.
//!
//! **Reproceso.** [`CommandSpec::stdin_data`] alimenta un texto por la
//! entrada estándar del proceso: permite reprocesar la salida capturada
//! de un comando previo con otra herramienta, sin volver a correr el
//! comando original.
//!
//! El crate es agnóstico de frontend: el proceso y sus lectores corren
//! en hilos; el consumidor (shell GPUI o TUI) drena el canal cuando
//! quiere — sin `async`, sin acoplarse a ningún runtime.

#![forbid(unsafe_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// Qué ejecutar: una línea de comandos, en un directorio, con un shell.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    /// La línea completa — se pasa como `shell -c "<line>"`.
    pub line: String,
    /// Directorio de trabajo del proceso.
    pub cwd: String,
    /// Programa de shell — `"bash"`, `"sh"`, `"fish"`…
    pub shell: String,
    /// Tope de bytes a capturar; `0` = sin límite. Pasado el tope, la
    /// salida se descarta (se emite [`RunEvent::Truncated`]).
    pub capture_limit: usize,
    /// Texto a alimentar por stdin — para reprocesar una salida previa.
    pub stdin_data: Option<String>,
}

impl CommandSpec {
    /// Spec con `bash` como shell, sin límite ni stdin.
    pub fn bash(line: impl Into<String>, cwd: impl Into<String>) -> Self {
        Self {
            line: line.into(),
            cwd: cwd.into(),
            shell: "bash".into(),
            capture_limit: 0,
            stdin_data: None,
        }
    }

    /// Fija el tope de captura en bytes (encadenable).
    pub fn with_limit(mut self, bytes: usize) -> Self {
        self.capture_limit = bytes;
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
    /// El proceso, compartido con su hilo coordinador para poder matarlo.
    child: Arc<Mutex<Option<Child>>>,
}

impl RunHandle {
    /// Mata el proceso (envía la señal de terminación). No hace nada si
    /// el proceso ya terminó o nunca llegó a lanzarse.
    pub fn kill(&self) {
        if let Ok(mut guard) = self.child.lock() {
            if let Some(c) = guard.as_mut() {
                let _ = c.kill();
            }
        }
    }

    /// Drena todos los eventos disponibles ahora mismo, sin bloquear.
    /// Marca el asa como terminada al ver un evento terminal.
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

    /// Bloquea hasta que el proceso termine y devuelve todos sus eventos
    /// en orden. Pensado para tests y para usos sincrónicos.
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

/// Lanza un hilo lector de un flujo. Cuenta los bytes contra `counter`;
/// pasado `limit` emite `Truncated` una vez (vía `announced`) y descarta
/// el resto, pero **sigue drenando** el pipe para no bloquear al proceso.
fn spawn_reader<R: Read + Send + 'static>(
    stream: R,
    tx: Sender<RunEvent>,
    make: fn(String) -> RunEvent,
    limit: usize,
    counter: Arc<AtomicUsize>,
    announced: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            let total = counter.fetch_add(line.len() + 1, Ordering::Relaxed) + line.len() + 1;
            if limit != 0 && total > limit {
                if !announced.swap(true, Ordering::Relaxed) {
                    let _ = tx.send(RunEvent::Truncated);
                }
                continue; // descarta, pero sigue leyendo el pipe
            }
            if tx.send(make(line)).is_err() {
                break;
            }
        }
    })
}

/// Lanza `spec` y devuelve un [`RunHandle`] desde el que drenar la
/// salida. La función vuelve de inmediato: el proceso corre en hilos.
pub fn run(spec: &CommandSpec) -> RunHandle {
    let (tx, rx) = mpsc::channel();
    let spec = spec.clone();
    let child_cell: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    let cell = Arc::clone(&child_cell);

    std::thread::spawn(move || {
        let stdin_mode = if spec.stdin_data.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        };
        let spawned = Command::new(&spec.shell)
            .arg("-c")
            .arg(&spec.line)
            .current_dir(&spec.cwd)
            .stdin(stdin_mode)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(RunEvent::Failed(e.to_string()));
                return;
            }
        };

        // Si hay datos para reprocesar, se escriben por stdin en su
        // propio hilo (la escritura puede bloquear hasta que el proceso
        // consuma); al terminar, `stdin` se cierra → EOF.
        if let Some(data) = spec.stdin_data.clone() {
            if let Some(mut stdin) = child.stdin.take() {
                std::thread::spawn(move || {
                    let _ = stdin.write_all(data.as_bytes());
                });
            }
        }

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        // Comparte el proceso para que `RunHandle::kill` pueda alcanzarlo.
        if let Ok(mut g) = cell.lock() {
            *g = Some(child);
        }

        // Contador de bytes compartido: el tope vale para stdout+stderr.
        let counter = Arc::new(AtomicUsize::new(0));
        let announced = Arc::new(AtomicBool::new(false));
        let limit = spec.capture_limit;

        let out_reader = stdout.map(|s| {
            spawn_reader(
                s,
                tx.clone(),
                RunEvent::Stdout,
                limit,
                Arc::clone(&counter),
                Arc::clone(&announced),
            )
        });
        let err_reader = stderr.map(|s| {
            spawn_reader(
                s,
                tx.clone(),
                RunEvent::Stderr,
                limit,
                Arc::clone(&counter),
                Arc::clone(&announced),
            )
        });

        // Los lectores terminan cuando el proceso cierra sus pipes —sea
        // por fin natural o por `kill`—; recién entonces se cosecha.
        if let Some(h) = out_reader {
            let _ = h.join();
        }
        if let Some(h) = err_reader {
            let _ = h.join();
        }
        let code = cell
            .lock()
            .ok()
            .and_then(|mut g| g.as_mut().and_then(|c| c.wait().ok()))
            .and_then(|s| s.code())
            .unwrap_or(-1);
        let _ = tx.send(RunEvent::Exited(code));
    });

    RunHandle { rx, finished: false, child: child_cell }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sh` está en cualquier entorno POSIX — más portable que bash
    /// para los tests.
    fn sh(line: &str) -> CommandSpec {
        CommandSpec { shell: "sh".into(), ..CommandSpec::bash(line, ".") }
    }

    #[test]
    fn captures_stdout_and_exit_code() {
        let mut h = run(&sh("echo hola-mundo"));
        let events = h.wait_all();
        assert!(events.contains(&RunEvent::Stdout("hola-mundo".into())));
        assert!(events.contains(&RunEvent::Exited(0)));
        assert!(h.is_finished());
    }

    #[test]
    fn captures_stderr() {
        let mut h = run(&sh("echo problema 1>&2"));
        let events = h.wait_all();
        assert!(events.contains(&RunEvent::Stderr("problema".into())));
    }

    #[test]
    fn nonzero_exit_is_reported() {
        let mut h = run(&sh("exit 3"));
        let events = h.wait_all();
        assert!(events.contains(&RunEvent::Exited(3)));
    }

    #[test]
    fn multiple_output_lines_arrive_in_order() {
        let mut h = run(&sh("echo uno; echo dos; echo tres"));
        let lines: Vec<String> = h
            .wait_all()
            .into_iter()
            .filter_map(|e| match e {
                RunEvent::Stdout(l) => Some(l),
                _ => None,
            })
            .collect();
        assert_eq!(lines, vec!["uno", "dos", "tres"]);
    }

    #[test]
    fn pipes_run_through_the_shell() {
        let mut h = run(&sh("printf 'b\\na\\nc\\n' | sort"));
        let lines: Vec<String> = h
            .wait_all()
            .into_iter()
            .filter_map(|e| match e {
                RunEvent::Stdout(l) => Some(l),
                _ => None,
            })
            .collect();
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn missing_shell_fails_gracefully() {
        let spec = CommandSpec { shell: "/no/existe/shell-xyz".into(), ..sh("echo x") };
        let mut h = run(&spec);
        let events = h.wait_all();
        assert!(matches!(events.first(), Some(RunEvent::Failed(_))));
    }

    #[test]
    fn terminal_event_detection() {
        assert!(RunEvent::Exited(0).is_terminal());
        assert!(RunEvent::Failed("x".into()).is_terminal());
        assert!(!RunEvent::Stdout("x".into()).is_terminal());
        assert!(!RunEvent::Truncated.is_terminal());
    }

    #[test]
    fn kill_stops_a_long_running_process() {
        let mut h = run(&sh("sleep 30"));
        std::thread::sleep(std::time::Duration::from_millis(250));
        h.kill();
        let events = h.wait_all();
        assert!(events.last().map(|e| e.is_terminal()).unwrap_or(false));
        assert!(h.is_finished());
    }

    #[test]
    fn capture_limit_truncates_but_process_finishes() {
        // 20.000 líneas, pero la captura se corta a ~400 bytes.
        let mut h = run(&sh("seq 1 20000").with_limit(400));
        let events = h.wait_all();
        // Se anunció el truncado…
        assert!(events.contains(&RunEvent::Truncated));
        // …pero el proceso terminó normal (no se bloqueó).
        assert!(events.contains(&RunEvent::Exited(0)));
        // Y lo capturado quedó acotado.
        let captured = events
            .iter()
            .filter(|e| matches!(e, RunEvent::Stdout(_)))
            .count();
        assert!(captured < 20000, "la salida quedó acotada");
    }

    #[test]
    fn no_limit_captures_everything() {
        let mut h = run(&sh("seq 1 500")); // capture_limit = 0
        let events = h.wait_all();
        assert!(!events.contains(&RunEvent::Truncated));
        let n = events.iter().filter(|e| matches!(e, RunEvent::Stdout(_))).count();
        assert_eq!(n, 500);
    }

    #[test]
    fn stdin_data_is_fed_to_the_process() {
        // `cat` devuelve por stdout lo que recibe por stdin — es el
        // reproceso más simple: tomar una salida y pasarla a otro filtro.
        let mut h = run(&sh("cat").with_stdin("alfa\nbeta\ngamma"));
        let lines: Vec<String> = h
            .wait_all()
            .into_iter()
            .filter_map(|e| match e {
                RunEvent::Stdout(l) => Some(l),
                _ => None,
            })
            .collect();
        assert_eq!(lines, vec!["alfa", "beta", "gamma"]);
    }

    #[test]
    fn stdin_data_reprocessed_by_a_filter() {
        let mut h = run(&sh("grep beta").with_stdin("alfa\nbeta\nbetabel\ngamma"));
        let lines: Vec<String> = h
            .wait_all()
            .into_iter()
            .filter_map(|e| match e {
                RunEvent::Stdout(l) => Some(l),
                _ => None,
            })
            .collect();
        assert_eq!(lines, vec!["beta", "betabel"]);
    }
}
