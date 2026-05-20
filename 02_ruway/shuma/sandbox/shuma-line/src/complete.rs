//! Autocompletado — sugerencias inteligentes según la posición del cursor.
//!
//! El motor decide *qué* se está escribiendo (un comando, un flag o una
//! ruta) mirando la estructura de la línea, y delega la búsqueda de
//! candidatos concretos en una [`CompletionSource`] que el frontend
//! provee (escaneo del `PATH`, del sistema de archivos, etc.).

use serde::{Deserialize, Serialize};

use crate::dialect::Dialect;
use crate::lexer::tokenize;
use crate::token::TokenKind;

/// Origen de candidatos concretos — lo implementa el frontend, que sí
/// conoce el sistema (el `PATH`, el disco). El motor de `shuma-line` se
/// mantiene agnóstico.
pub trait CompletionSource {
    /// Nombres de comandos disponibles (típicamente, escaneo del `PATH`).
    fn commands(&self) -> Vec<String>;
    /// Rutas de archivo que empiezan con `prefix`.
    fn paths(&self, prefix: &str) -> Vec<String>;
}

/// Qué clase de cosa se está completando.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompletionKind {
    /// El nombre de un comando.
    Command,
    /// Una opción de un comando.
    Flag,
    /// Una ruta del sistema de archivos.
    Path,
}

/// El resultado de un intento de autocompletado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completion {
    pub kind: CompletionKind,
    /// Candidatos, ordenados y sin repetir.
    pub candidates: Vec<String>,
    /// Inicio del rango de bytes a reemplazar al aceptar un candidato.
    pub replace_start: usize,
    /// Fin del rango de bytes a reemplazar.
    pub replace_end: usize,
}

impl Completion {
    /// `true` si no hay ningún candidato.
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

/// Pistas de flags por comando — un diccionario mínimo de los comandos
/// más usados. La fuente real de un frontend puede ampliarlo.
pub fn flag_hints(command: &str) -> &'static [&'static str] {
    match command {
        "ls" => &["-l", "-a", "-la", "-lh", "-R", "-t", "--all", "--color", "--human-readable"],
        "grep" => &["-i", "-v", "-r", "-n", "-E", "-l", "-c", "--color", "--include"],
        "rm" => &["-r", "-f", "-rf", "-i", "-v"],
        "cp" => &["-r", "-a", "-v", "-p", "-u"],
        "mv" => &["-f", "-i", "-n", "-v"],
        "cargo" => &["--release", "--workspace", "--all-features", "-p", "--bin", "--example"],
        "git" => &["--version", "--help", "-C"],
        "docker" => &["-d", "-it", "--name", "--restart", "-p", "-e", "-v", "--rm"],
        "ps" => &["-e", "-f", "-aux", "-u"],
        "tar" => &["-c", "-x", "-z", "-v", "-f", "-czf", "-xzf"],
        "curl" => &["-s", "-L", "-o", "-O", "-X", "-H", "-d"],
        _ => &[],
    }
}

/// Calcula el autocompletado para `line` con el cursor en `cursor`
/// (offset de byte). Nunca entra en pánico si `cursor` cae en mitad de
/// un carácter: se ajusta al límite válido anterior.
pub fn complete(
    line: &str,
    cursor: usize,
    dialect: Dialect,
    source: &dyn CompletionSource,
) -> Completion {
    let mut cursor = cursor.min(line.len());
    while cursor > 0 && !line.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let tokens = tokenize(line, dialect);

    // Token que se está editando: aquel cuyo contenido llega al cursor.
    let word_token = tokens
        .iter()
        .find(|t| t.start < cursor && cursor <= t.end && t.kind.is_content());
    let (prefix, repl_start, repl_end) = match word_token {
        Some(t) => (&line[t.start..cursor], t.start, cursor),
        None => ("", cursor, cursor),
    };
    let word_start = repl_start;

    // Recorre los tokens previos a la palabra para saber si la etapa
    // actual ya tiene comando (→ estamos en posición de argumento).
    let mut stage_command: Option<String> = None;
    let mut has_command = false;
    for t in &tokens {
        if t.end > word_start {
            break;
        }
        match t.kind {
            TokenKind::Pipe | TokenKind::Operator => {
                stage_command = None;
                has_command = false;
            }
            TokenKind::Command => {
                stage_command = Some(t.text.clone());
                has_command = true;
            }
            _ => {}
        }
    }

    let (kind, mut candidates) = if !has_command {
        let cs = source
            .commands()
            .into_iter()
            .filter(|c| c.starts_with(prefix))
            .collect();
        (CompletionKind::Command, cs)
    } else if prefix.starts_with('-') {
        let hints = stage_command.as_deref().map(flag_hints).unwrap_or(&[]);
        let cs = hints
            .iter()
            .filter(|f| f.starts_with(prefix))
            .map(|s| s.to_string())
            .collect();
        (CompletionKind::Flag, cs)
    } else {
        (CompletionKind::Path, source.paths(prefix))
    };

    candidates.sort();
    candidates.dedup();
    candidates.truncate(200);
    Completion { kind, candidates, replace_start: repl_start, replace_end: repl_end }
}

/// Fuente de candidatos con listas fijas — útil para tests y para un
/// arranque sin escaneo del sistema.
#[derive(Debug, Clone, Default)]
pub struct StaticSource {
    pub commands: Vec<String>,
    pub paths: Vec<String>,
}

impl CompletionSource for StaticSource {
    fn commands(&self) -> Vec<String> {
        self.commands.clone()
    }
    fn paths(&self, prefix: &str) -> Vec<String> {
        self.paths
            .iter()
            .filter(|p| p.starts_with(prefix))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> StaticSource {
        StaticSource {
            commands: vec![
                "ls".into(),
                "lsblk".into(),
                "grep".into(),
                "git".into(),
                "cargo".into(),
            ],
            paths: vec![
                "Cargo.toml".into(),
                "Cargo.lock".into(),
                "src/".into(),
                "README.md".into(),
            ],
        }
    }

    fn complete_at(line: &str, cursor: usize) -> Completion {
        complete(line, cursor, Dialect::Bash, &source())
    }

    #[test]
    fn completes_command_names_from_prefix() {
        let c = complete_at("ls", 2);
        assert_eq!(c.kind, CompletionKind::Command);
        assert_eq!(c.candidates, vec!["ls", "lsblk"]);
        assert_eq!((c.replace_start, c.replace_end), (0, 2));
    }

    #[test]
    fn completes_flags_for_the_stage_command() {
        let c = complete_at("ls -l", 5);
        assert_eq!(c.kind, CompletionKind::Flag);
        assert!(c.candidates.contains(&"-l".to_string()));
        assert!(c.candidates.contains(&"-la".to_string()));
        assert!(c.candidates.iter().all(|f| f.starts_with("-l")));
    }

    #[test]
    fn completes_paths_in_argument_position() {
        let c = complete_at("cat Cargo", 9);
        assert_eq!(c.kind, CompletionKind::Path);
        assert_eq!(c.candidates, vec!["Cargo.lock", "Cargo.toml"]);
    }

    #[test]
    fn completes_command_after_a_pipe() {
        // Tras `| g`, se completa un comando nuevo, no una ruta.
        let c = complete_at("cat f | g", 9);
        assert_eq!(c.kind, CompletionKind::Command);
        assert_eq!(c.candidates, vec!["git", "grep"]);
    }

    #[test]
    fn empty_line_offers_all_commands() {
        let c = complete_at("", 0);
        assert_eq!(c.kind, CompletionKind::Command);
        assert_eq!(c.candidates.len(), 5);
    }

    #[test]
    fn completing_in_whitespace_starts_a_fresh_word() {
        // Cursor tras `cargo ` → posición de argumento, prefijo vacío.
        let c = complete_at("cargo ", 6);
        assert_eq!(c.kind, CompletionKind::Path);
        assert_eq!((c.replace_start, c.replace_end), (6, 6));
    }

    #[test]
    fn flag_completion_knows_the_command() {
        let c = complete_at("cargo --re", 10);
        assert_eq!(c.kind, CompletionKind::Flag);
        assert_eq!(c.candidates, vec!["--release"]);
    }

    #[test]
    fn cursor_past_end_is_clamped() {
        let c = complete_at("gi", 999);
        assert_eq!(c.candidates, vec!["git"]);
    }
}
