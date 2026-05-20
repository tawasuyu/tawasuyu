//! `shuma-session` — la sesión de trabajo del shell.
//!
//! El shell no es una sucesión suelta de comandos: trabaja *dentro de
//! una sesión*. Una [`WorkSession`] contiene:
//!
//! - el **directorio actual** — que es además el identificador de
//!   aislamiento (cada directorio es un contexto separado);
//! - el **historial** de comandos ejecutados, cada uno con su salida y
//!   su estado ([`CommandRun`]);
//! - los **grupos** de comandos guardados y reutilizables
//!   ([`CommandGroup`]).
//!
//! Modelo puro y agnóstico: la ejecución real la hace `shuma-exec`, el
//! tiempo lo inyecta el caller. Determinista y testeable.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Identificador de un comando dentro de su sesión.
pub type RunId = u64;

/// Estado de un comando ejecutado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    /// Ejecutándose; su salida sigue llegando.
    Running,
    /// Terminó con código 0.
    Ok,
    /// Terminó con código distinto de 0, o no pudo lanzarse.
    Failed,
}

/// Un comando ejecutado: la línea, el directorio, el estado y la salida.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRun {
    pub id: RunId,
    /// La línea de comandos tal como se escribió.
    pub line: String,
    /// El directorio en que se ejecutó.
    pub cwd: String,
    pub status: RunStatus,
    /// Código de salida, una vez terminado.
    pub exit_code: Option<i32>,
    /// Salida combinada (stdout + stderr), una línea por elemento.
    pub output: Vec<String>,
    /// Segundo Unix en que arrancó.
    pub started_at: u64,
    /// Segundo Unix en que terminó.
    pub finished_at: Option<u64>,
}

impl CommandRun {
    /// `true` si el comando sigue corriendo.
    pub fn is_running(&self) -> bool {
        self.status == RunStatus::Running
    }

    /// Cantidad de líneas de salida.
    pub fn line_count(&self) -> usize {
        self.output.len()
    }
}

/// Un grupo de comandos guardado para reutilizar — una receta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandGroup {
    pub name: String,
    /// Las líneas de comando, en orden de ejecución.
    pub lines: Vec<String>,
}

/// La sesión de trabajo: directorio actual + historial + grupos.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkSession {
    pub name: String,
    cwd: String,
    history: Vec<CommandRun>,
    groups: Vec<CommandGroup>,
    next_id: RunId,
}

/// FNV-1a de 64 bits — base del identificador de aislamiento.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

impl WorkSession {
    /// Abre una sesión con un nombre y un directorio inicial.
    pub fn new(name: impl Into<String>, cwd: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            cwd: cwd.into(),
            history: Vec::new(),
            groups: Vec::new(),
            next_id: 1,
        }
    }

    /// Directorio actual de la sesión.
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Cambia el directorio actual — y con él, el contexto de aislamiento.
    pub fn set_cwd(&mut self, cwd: impl Into<String>) {
        self.cwd = cwd.into();
    }

    /// Identificador de aislamiento del directorio actual: un hash corto
    /// y estable del `cwd`. Cada directorio es un contexto distinto, así
    /// que el id cambia al hacer `cd`.
    pub fn isolation_id(&self) -> String {
        format!("{:012x}", fnv1a(self.cwd.as_bytes()) & 0xffff_ffff_ffff)
    }

    // --- Historial de comandos ---

    /// Registra el inicio de un comando (estado `Running`) en el `cwd`
    /// actual. Devuelve su id.
    pub fn begin_run(&mut self, line: impl Into<String>, now: u64) -> RunId {
        let id = self.next_id;
        self.next_id += 1;
        self.history.push(CommandRun {
            id,
            line: line.into(),
            cwd: self.cwd.clone(),
            status: RunStatus::Running,
            exit_code: None,
            output: Vec::new(),
            started_at: now,
            finished_at: None,
        });
        id
    }

    pub fn run(&self, id: RunId) -> Option<&CommandRun> {
        self.history.iter().find(|r| r.id == id)
    }

    fn run_mut(&mut self, id: RunId) -> Option<&mut CommandRun> {
        self.history.iter_mut().find(|r| r.id == id)
    }

    /// Añade una línea de salida a un comando en curso.
    pub fn append_output(&mut self, id: RunId, line: impl Into<String>) {
        if let Some(r) = self.run_mut(id) {
            r.output.push(line.into());
        }
    }

    /// Marca un comando como terminado con su código de salida.
    pub fn finish_run(&mut self, id: RunId, exit_code: i32, now: u64) {
        if let Some(r) = self.run_mut(id) {
            r.exit_code = Some(exit_code);
            r.status = if exit_code == 0 { RunStatus::Ok } else { RunStatus::Failed };
            r.finished_at = Some(now);
        }
    }

    /// Historial completo, del más antiguo al más reciente.
    pub fn history(&self) -> &[CommandRun] {
        &self.history
    }

    /// Comandos que siguen corriendo.
    pub fn running(&self) -> Vec<RunId> {
        self.history
            .iter()
            .filter(|r| r.is_running())
            .map(|r| r.id)
            .collect()
    }

    /// Vacía el historial (no toca los grupos ni el `cwd`).
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    // --- Grupos reutilizables ---

    /// Guarda un grupo de comandos. Si ya existe uno con ese nombre, lo
    /// reemplaza.
    pub fn save_group(&mut self, name: impl Into<String>, lines: Vec<String>) {
        let name = name.into();
        self.groups.retain(|g| g.name != name);
        self.groups.push(CommandGroup { name, lines });
    }

    /// Guarda como grupo las líneas de los últimos `n` comandos del
    /// historial — la forma natural de "convertir lo que acabo de hacer
    /// en una receta".
    pub fn save_recent_as_group(&mut self, name: impl Into<String>, n: usize) {
        let lines: Vec<String> = self
            .history
            .iter()
            .rev()
            .take(n)
            .rev()
            .map(|r| r.line.clone())
            .collect();
        self.save_group(name, lines);
    }

    pub fn groups(&self) -> &[CommandGroup] {
        &self.groups
    }

    pub fn group(&self, name: &str) -> Option<&CommandGroup> {
        self.groups.iter().find(|g| g.name == name)
    }

    /// Quita un grupo. `true` si existía.
    pub fn remove_group(&mut self, name: &str) -> bool {
        let before = self.groups.len();
        self.groups.retain(|g| g.name != name);
        self.groups.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolation_id_follows_the_directory() {
        let mut s = WorkSession::new("trabajo", "/home/sergio/brahman");
        let id_a = s.isolation_id();
        s.set_cwd("/tmp");
        let id_b = s.isolation_id();
        assert_ne!(id_a, id_b, "cd cambia el contexto de aislamiento");
        // Estable: el mismo directorio da el mismo id.
        s.set_cwd("/home/sergio/brahman");
        assert_eq!(s.isolation_id(), id_a);
    }

    #[test]
    fn a_run_records_its_directory() {
        let mut s = WorkSession::new("t", "/home");
        let id = s.begin_run("ls -la", 1000);
        assert_eq!(s.run(id).unwrap().cwd, "/home");
        assert_eq!(s.run(id).unwrap().status, RunStatus::Running);
    }

    #[test]
    fn output_accumulates_and_run_finishes() {
        let mut s = WorkSession::new("t", "/home");
        let id = s.begin_run("echo hola", 1000);
        s.append_output(id, "hola");
        s.finish_run(id, 0, 1001);
        let r = s.run(id).unwrap();
        assert_eq!(r.output, vec!["hola"]);
        assert_eq!(r.status, RunStatus::Ok);
        assert_eq!(r.exit_code, Some(0));
        assert_eq!(r.finished_at, Some(1001));
    }

    #[test]
    fn nonzero_exit_marks_failed() {
        let mut s = WorkSession::new("t", "/home");
        let id = s.begin_run("false", 0);
        s.finish_run(id, 1, 1);
        assert_eq!(s.run(id).unwrap().status, RunStatus::Failed);
    }

    #[test]
    fn running_lists_unfinished_commands() {
        let mut s = WorkSession::new("t", "/home");
        let a = s.begin_run("sleep 1", 0);
        let b = s.begin_run("sleep 2", 0);
        s.finish_run(a, 0, 1);
        assert_eq!(s.running(), vec![b]);
    }

    #[test]
    fn save_and_recall_a_group() {
        let mut s = WorkSession::new("t", "/home");
        s.save_group("deploy", vec!["cargo build".into(), "scp target host:/srv".into()]);
        assert_eq!(s.group("deploy").unwrap().lines.len(), 2);
        // Guardar con el mismo nombre reemplaza.
        s.save_group("deploy", vec!["echo nuevo".into()]);
        assert_eq!(s.group("deploy").unwrap().lines, vec!["echo nuevo"]);
    }

    #[test]
    fn save_recent_history_as_a_group() {
        let mut s = WorkSession::new("t", "/home");
        for line in ["git add .", "git commit", "git push"] {
            s.begin_run(line, 0);
        }
        s.save_recent_as_group("publicar", 2);
        // Los 2 últimos, en orden cronológico.
        assert_eq!(s.group("publicar").unwrap().lines, vec!["git commit", "git push"]);
    }

    #[test]
    fn remove_group() {
        let mut s = WorkSession::new("t", "/home");
        s.save_group("x", vec!["echo x".into()]);
        assert!(s.remove_group("x"));
        assert!(!s.remove_group("x"));
    }
}
