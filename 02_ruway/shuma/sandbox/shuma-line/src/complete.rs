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
    /// Banderas conocidas para `command`. Por defecto delega en la tabla
    /// estática [`flag_hints`] (que cubre los binarios más usados). El
    /// frontend puede sobreescribir el método para mergear con un DB
    /// personalizado (p. ej. `~/.config/shuma/completions/<cmd>.toml`).
    fn flags(&self, command: &str) -> Vec<String> {
        flag_hints(command).iter().map(|s| s.to_string()).collect()
    }
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

/// Flags universales — `--help` y `-h` los reconoce casi todo binario
/// POSIX/GNU/clap-style. Se agregan siempre al final de las sugerencias.
const UNIVERSAL_FLAGS: &[&str] = &["--help", "-h"];

/// Pistas de flags por comando — diccionario *static* de los comandos
/// más usados en una shell de desarrollo. La fuente del frontend puede
/// extenderlo con un DB cargado en runtime (p. ej. desde
/// `~/.config/shuma/completions/`).
pub fn flag_hints(command: &str) -> &'static [&'static str] {
    match command {
        // --- coreutils ---
        "ls" => &[
            "-l", "-a", "-la", "-lh", "-A", "-R", "-r", "-t", "-S", "-d", "-1", "-F",
            "--all", "--almost-all", "--color", "--color=always", "--color=auto",
            "--color=never", "--human-readable", "--group-directories-first",
            "--sort=time", "--sort=size", "--sort=name", "--reverse",
        ],
        "cat" => &["-A", "-b", "-E", "-n", "-s", "-T", "-v", "--number", "--show-ends"],
        "grep" => &[
            "-i", "-v", "-r", "-R", "-n", "-E", "-F", "-l", "-c", "-w", "-x", "-o",
            "-A", "-B", "-C", "--color", "--color=always", "--include", "--exclude",
            "--exclude-dir", "--binary-files=without-match",
        ],
        "sed" => &["-e", "-f", "-i", "-n", "-r", "-E", "--in-place"],
        "awk" => &["-F", "-v", "-f", "-W"],
        "find" => &[
            "-name", "-iname", "-type", "-mtime", "-newer", "-size", "-maxdepth",
            "-mindepth", "-prune", "-print", "-exec", "-delete", "-empty", "-not",
            "-and", "-or", "-path", "-regex",
        ],
        "rm" => &["-r", "-f", "-rf", "-i", "-v", "-d", "--recursive", "--force", "--interactive"],
        "cp" => &["-r", "-a", "-v", "-p", "-u", "-f", "-i", "-n", "--recursive", "--archive"],
        "mv" => &["-f", "-i", "-n", "-v", "--no-clobber"],
        "mkdir" => &["-p", "-v", "-m", "--parents"],
        "head" => &["-n", "-c", "-q", "-v"],
        "tail" => &["-n", "-c", "-f", "-F", "-q", "-v", "--follow", "--retry"],
        "wc" => &["-c", "-l", "-w", "-m", "-L"],
        "sort" => &["-n", "-r", "-u", "-k", "-t", "-f", "-h", "-V", "--unique", "--reverse"],
        "uniq" => &["-c", "-d", "-u", "-i", "-f", "-s"],
        "du" => &["-h", "-s", "-a", "-c", "-d", "-x", "--max-depth", "--summarize"],
        "df" => &["-h", "-T", "-i", "-x", "--type", "--human-readable"],
        "ps" => &["-e", "-f", "-aux", "-u", "-o", "-p", "--ppid"],
        "kill" => &["-9", "-15", "-STOP", "-CONT", "-HUP", "-INT", "-l", "-s"],
        "tar" => &[
            "-c", "-x", "-z", "-j", "-J", "-v", "-f", "-t", "-C",
            "-czf", "-xzf", "-tzf", "-cjf", "-xjf",
            "--create", "--extract", "--list", "--gzip", "--bzip2", "--xz",
        ],
        "curl" => &[
            "-s", "-S", "-L", "-o", "-O", "-X", "-H", "-d", "-D", "-i", "-I", "-v", "-f", "-k",
            "--silent", "--show-error", "--location", "--output", "--remote-name",
            "--request", "--header", "--data", "--insecure", "--fail",
        ],
        "wget" => &[
            "-q", "-O", "-c", "-r", "-l", "--quiet", "--output-document", "--continue",
            "--recursive", "--no-check-certificate",
        ],
        "ssh" => &["-i", "-p", "-l", "-L", "-R", "-D", "-N", "-T", "-X", "-Y", "-A", "-J", "-J"],
        "scp" => &["-r", "-P", "-i", "-p", "-q", "-C"],
        "rsync" => &[
            "-a", "-v", "-z", "-h", "-r", "-n", "--archive", "--verbose", "--compress",
            "--dry-run", "--delete", "--exclude", "--progress",
        ],

        // --- cargo / rust ---
        "cargo" => &[
            "--release", "--workspace", "--all-features", "--no-default-features",
            "--features", "-p", "--package", "--bin", "--bins", "--example", "--examples",
            "--lib", "--test", "--tests", "--bench", "--benches", "--target", "--target-dir",
            "--manifest-path", "--frozen", "--locked", "--offline", "-v", "-vv", "--quiet",
            "--color=always", "--message-format=json",
        ],
        "rustup" => &["--version", "--verbose", "--quiet", "--toolchain"],
        "rustc" => &[
            "--edition", "--crate-type", "--emit", "-O", "-g", "-C", "-Z", "--target",
            "-L", "-l", "--cfg", "--print",
        ],

        // --- git ---
        "git" => &["-C", "-c", "-p", "--paginate", "--no-pager", "--version", "--git-dir", "--work-tree"],

        // --- contenedores / k8s ---
        "docker" => &[
            "-d", "-it", "--rm", "--name", "--restart", "-p", "-e", "-v",
            "--network", "--volume", "--env", "--env-file", "--cpus", "--memory",
        ],
        "podman" => &["-d", "-it", "--rm", "--name", "-p", "-e", "-v", "--network", "--pod"],
        "kubectl" => &[
            "-n", "--namespace", "-o", "--output", "-w", "--watch", "-f", "--filename",
            "--context", "-l", "--selector", "--all-namespaces", "-A",
        ],
        "systemctl" => &[
            "--user", "--system", "--now", "--no-pager", "--full", "-l", "-r", "-a",
            "--state", "--type", "--failed",
        ],
        "journalctl" => &[
            "-u", "-f", "-r", "-n", "-k", "-b", "--since", "--until", "--user-unit",
            "--no-pager", "-p", "--priority",
        ],

        // --- desarrollo ---
        "make" => &["-j", "-C", "-f", "-n", "-B", "-s", "--jobs", "--always-make"],
        "ninja" => &["-j", "-C", "-n", "-v", "-t"],
        "python" => &["-c", "-m", "-u", "-V", "-O", "-OO", "-i", "--version"],
        "python3" => &["-c", "-m", "-u", "-V", "-O", "-OO", "-i", "--version"],
        "node" => &["-e", "-v", "-p", "--inspect", "--inspect-brk", "--version"],
        "deno" => &["run", "test", "fmt", "lint", "-A", "--allow-net", "--allow-read", "--allow-write"],
        "go" => &["build", "run", "test", "mod", "get", "fmt", "vet", "-race", "-tags"],

        // --- vim / editores ---
        "vim" => &["-c", "-O", "-o", "-p", "-R", "-d", "-u", "-N", "+"],
        "nvim" => &["-c", "-O", "-o", "-p", "-R", "-d", "-u", "-N", "+", "--headless"],
        "hx" => &["--tutor", "--health", "--config", "-V", "--version"],
        "code" => &["-r", "-n", "-g", "-d", "--reuse-window", "--new-window", "--goto", "--diff"],

        // --- proceso / debug ---
        "strace" => &["-e", "-f", "-o", "-p", "-c", "-y", "-tt", "-T", "-s"],
        "ltrace" => &["-e", "-f", "-o", "-p", "-c"],
        "gdb" => &["-q", "-c", "--args", "-ex", "-batch", "--tui"],
        "perf" => &["record", "report", "stat", "top", "-F", "-g", "-p", "-e"],

        // --- shuma / brahman ---
        "shuma" => &[
            "workspace", "run", "pipeline", "discern", "capabilities",
            "--socket", "--json", "--verbose",
        ],

        _ => &[],
    }
}

/// Extiende cualquier lista de flags con los universales (`--help`/`-h`)
/// si todavía no están presentes. Lo usa el motor cuando filtra los
/// candidatos para `prefix`.
fn extend_with_universal(mut flags: Vec<String>) -> Vec<String> {
    for u in UNIVERSAL_FLAGS {
        let s = (*u).to_string();
        if !flags.iter().any(|f| f == &s) {
            flags.push(s);
        }
    }
    flags
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

    let (kind, mut candidates, repl_start_final) = if !has_command {
        let cs = source
            .commands()
            .into_iter()
            .filter(|c| c.starts_with(prefix))
            .collect();
        (CompletionKind::Command, cs, repl_start)
    } else if prefix.starts_with('-') {
        // Caso `--foo=<...>`: tras `=`, el cursor está completando el
        // *valor* del flag, no otro flag. Lo más útil hoy es path
        // completion (cubre `--config=`, `--output=`, etc.). En el futuro
        // podríamos consultar al source por tipos de valor por flag.
        if let Some(eq) = prefix.find('=') {
            let value_prefix = &prefix[eq + 1..];
            let cs = source.paths(value_prefix);
            (CompletionKind::Path, cs, repl_start + eq + 1)
        } else {
            let hints = stage_command
                .as_deref()
                .map(|c| source.flags(c))
                .unwrap_or_default();
            let cs = extend_with_universal(hints)
                .into_iter()
                .filter(|f| f.starts_with(prefix))
                .collect();
            (CompletionKind::Flag, cs, repl_start)
        }
    } else {
        (CompletionKind::Path, source.paths(prefix), repl_start)
    };

    candidates.sort();
    candidates.dedup();
    candidates.truncate(200);
    Completion {
        kind,
        candidates,
        replace_start: repl_start_final,
        replace_end: repl_end,
    }
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

    #[test]
    fn universal_help_flags_always_suggested() {
        // Comando sin entrada en la tabla estática igualmente recibe -h/--help.
        let c = complete_at("foobar -", 8);
        assert_eq!(c.kind, CompletionKind::Flag);
        assert!(c.candidates.contains(&"-h".to_string()));
        assert!(c.candidates.contains(&"--help".to_string()));
    }

    #[test]
    fn equals_in_flag_switches_to_path_completion() {
        // `cargo --manifest-path=Car` debe completar paths a partir de `Car`,
        // reemplazando sólo el sufijo (no el flag completo).
        let c = complete_at("cargo --manifest-path=Car", 25);
        assert_eq!(c.kind, CompletionKind::Path);
        assert_eq!(c.candidates, vec!["Cargo.lock", "Cargo.toml"]);
        // El reemplazo arranca en la posición justo después del `=`.
        let s = "cargo --manifest-path=";
        assert_eq!(c.replace_start, s.len());
        assert_eq!(c.replace_end, 25);
    }

    #[test]
    fn source_can_override_flag_db() {
        // Una fuente custom puede ampliar el catálogo más allá de la
        // tabla estática (lo aprovecha el shell para cargar
        // ~/.config/shuma/completions/<cmd>.toml).
        #[derive(Default)]
        struct CustomSource {
            commands: Vec<String>,
        }
        impl CompletionSource for CustomSource {
            fn commands(&self) -> Vec<String> {
                self.commands.clone()
            }
            fn paths(&self, _: &str) -> Vec<String> {
                Vec::new()
            }
            fn flags(&self, command: &str) -> Vec<String> {
                if command == "mytool" {
                    vec!["--mytool-only".into(), "--verbose".into()]
                } else {
                    flag_hints(command).iter().map(|s| s.to_string()).collect()
                }
            }
        }
        let s = CustomSource { commands: vec!["mytool".into()] };
        let c = complete("mytool --m", 10, Dialect::Bash, &s);
        assert_eq!(c.kind, CompletionKind::Flag);
        assert!(c.candidates.contains(&"--mytool-only".to_string()));
    }

    #[test]
    fn after_pipe_help_flag_works_for_new_stage_command() {
        // `cargo build | grep -` → flags de grep, no de cargo.
        let c = complete_at("cargo build | grep -", 20);
        assert_eq!(c.kind, CompletionKind::Flag);
        // grep tiene -i; cargo no.
        assert!(c.candidates.iter().any(|f| f == "-i"));
        // El universal sigue ahí.
        assert!(c.candidates.contains(&"-h".to_string()));
    }
}
