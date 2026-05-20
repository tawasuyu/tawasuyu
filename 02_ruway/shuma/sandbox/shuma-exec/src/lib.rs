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
//! El crate es agnóstico de frontend: el proceso y sus lectores corren
//! en hilos; el consumidor (shell GPUI o TUI) drena el canal cuando
//! quiere — sin `async`, sin acoplarse a ningún runtime.

#![forbid(unsafe_code)]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// Qué ejecutar: una línea de comandos, en un directorio, con un shell.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    /// La línea completa — se pasa como `shell -c "<line>"`.
    pub line: String,
    /// Directorio de trabajo del proceso.
    pub cwd: String,
    /// Programa de shell — `"bash"`, `"sh"`, `"fish"`…
    pub shell: String,
}

impl CommandSpec {
    /// Spec con `bash` como shell.
    pub fn bash(line: impl Into<String>, cwd: impl Into<String>) -> Self {
        Self { line: line.into(), cwd: cwd.into(), shell: "bash".into() }
    }
}

/// Un evento de la ejecución de un comando.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    /// Una línea de salida estándar.
    Stdout(String),
    /// Una línea de salida de error.
    Stderr(String),
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
}

impl RunHandle {
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

/// Lanza `spec` y devuelve un [`RunHandle`] desde el que drenar la
/// salida. La función vuelve de inmediato: el proceso corre en hilos.
pub fn run(spec: &CommandSpec) -> RunHandle {
    let (tx, rx) = mpsc::channel();
    let spec = spec.clone();

    std::thread::spawn(move || {
        let spawned = Command::new(&spec.shell)
            .arg("-c")
            .arg(&spec.line)
            .current_dir(&spec.cwd)
            .stdin(Stdio::null())
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

        // Un hilo lector por flujo: stdout y stderr fluyen en paralelo.
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let out_reader = stdout.map(|s| {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(s).lines().map_while(Result::ok) {
                    if tx.send(RunEvent::Stdout(line)).is_err() {
                        break;
                    }
                }
            })
        });
        let err_reader = stderr.map(|s| {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(s).lines().map_while(Result::ok) {
                    if tx.send(RunEvent::Stderr(line)).is_err() {
                        break;
                    }
                }
            })
        });

        if let Some(h) = out_reader {
            let _ = h.join();
        }
        if let Some(h) = err_reader {
            let _ = h.join();
        }
        let code = child
            .wait()
            .ok()
            .and_then(|s| s.code())
            .unwrap_or(-1);
        let _ = tx.send(RunEvent::Exited(code));
    });

    RunHandle { rx, finished: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sh` está en cualquier entorno POSIX — más portable que bash
    /// para los tests.
    fn sh(line: &str) -> CommandSpec {
        CommandSpec { line: line.into(), cwd: ".".into(), shell: "sh".into() }
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
        let spec = CommandSpec {
            line: "echo x".into(),
            cwd: ".".into(),
            shell: "/no/existe/shell-xyz".into(),
        };
        let mut h = run(&spec);
        let events = h.wait_all();
        assert!(matches!(events.first(), Some(RunEvent::Failed(_))));
    }

    #[test]
    fn terminal_event_detection() {
        assert!(RunEvent::Exited(0).is_terminal());
        assert!(RunEvent::Failed("x".into()).is_terminal());
        assert!(!RunEvent::Stdout("x".into()).is_terminal());
    }
}
