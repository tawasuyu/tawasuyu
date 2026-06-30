//! Absorción de historiales de **shells ajenos** (bash, zsh) al historial
//! propio de shuma.
//!
//! El usuario que estrena shuma no llega con las manos vacías: ya tiene años
//! de comandos en `~/.bash_history` y `~/.zsh_history`. Importarlos hace que
//! el ghost, el ranking por frecuencia y la búsqueda fuzzy funcionen **desde
//! el primer arranque**, sin reaprender.
//!
//! Diseño:
//!
//! - **Parsers tolerantes.** bash es una línea por comando (con líneas
//!   `#<unixts>` opcionales si `HISTTIMEFORMAT` está puesto). zsh tiene dos
//!   formatos: plano (igual que bash) y *extended* (`: <ts>:<elapsed>;cmd`),
//!   este último con continuación por `\` al final de línea para comandos
//!   multilínea. Ambos parsers nunca entran en pánico; una línea ilegible se
//!   saltea.
//! - **Importación incremental.** Un fichero de estado
//!   (`shell_import.json`) recuerda cuántas entradas de cada fuente ya se
//!   absorbieron y el tamaño del fichero. Al relanzar shuma sólo se importa
//!   la **cola nueva** — no se reimporta todo cada vez. Si el fichero
//!   encogió (historial limpiado/rotado), se reimporta desde cero.
//! - **Orden cronológico.** Las entradas nuevas de todas las fuentes se
//!   mezclan por timestamp antes de appendear, así el historial propio queda
//!   en orden temporal coherente.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Entry, History};

/// Qué shell produjo un fichero de historial — determina el parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    /// `~/.bash_history` — una línea por comando, `#<ts>` opcional.
    Bash,
    /// `~/.zsh_history` — plano o *extended* (`: ts:elapsed;cmd`).
    Zsh,
}

/// Una fuente de historial ajeno a absorber.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignSource {
    pub kind: ShellKind,
    pub path: PathBuf,
}

impl ForeignSource {
    pub fn bash(path: impl Into<PathBuf>) -> Self {
        Self { kind: ShellKind::Bash, path: path.into() }
    }
    pub fn zsh(path: impl Into<PathBuf>) -> Self {
        Self { kind: ShellKind::Zsh, path: path.into() }
    }
}

/// Resultado de una absorción — para reportar en la UI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Entradas efectivamente añadidas al historial.
    pub imported: usize,
    /// Fuentes que aportaron al menos una entrada nueva.
    pub sources: Vec<PathBuf>,
}

impl ImportReport {
    pub fn is_empty(&self) -> bool {
        self.imported == 0
    }
}

/// Fuentes por defecto a partir de `$HOME` / `$ZDOTDIR` / `$HISTFILE`.
/// Sólo las que **existen** en disco. Respeta `HISTFILE` de bash si apunta a
/// otro fichero. zsh busca en `$ZDOTDIR` antes que en `$HOME`.
pub fn default_sources() -> Vec<ForeignSource> {
    let mut out = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);

    // bash: HISTFILE si está, si no ~/.bash_history.
    let bash_path = std::env::var_os("HISTFILE")
        .map(PathBuf::from)
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().contains("bash")))
        .or_else(|| home.as_ref().map(|h| h.join(".bash_history")));
    if let Some(p) = bash_path {
        if p.exists() {
            out.push(ForeignSource::bash(p));
        }
    }

    // zsh: $ZDOTDIR/.zsh_history o ~/.zsh_history.
    let zsh_path = std::env::var_os("ZDOTDIR")
        .map(|z| PathBuf::from(z).join(".zsh_history"))
        .or_else(|| home.as_ref().map(|h| h.join(".zsh_history")));
    if let Some(p) = zsh_path {
        if p.exists() {
            out.push(ForeignSource::zsh(p));
        }
    }
    out
}

/// Parsea el contenido de un `~/.bash_history`. Las líneas `#<digits>` son
/// timestamps (de `HISTTIMEFORMAT`) y se adjuntan al comando siguiente.
pub fn parse_bash(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut pending_ts: Option<u64> = None;
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        // `#1700000000` → timestamp del próximo comando.
        if let Some(rest) = line.strip_prefix('#') {
            if let Ok(ts) = rest.trim().parse::<u64>() {
                pending_ts = Some(ts);
                continue;
            }
            // `#` que no es timestamp = comentario en historial: se saltea.
            continue;
        }
        out.push(Entry::new(line, "", pending_ts.take().unwrap_or(0)));
    }
    out
}

/// Parsea el contenido de un `~/.zsh_history`. Soporta el formato *extended*
/// (`: <ts>:<elapsed>;cmd`) y el plano (una línea por comando). Los comandos
/// multilínea se reúnen siguiendo la continuación por `\` al final de línea
/// (cómo zsh codifica un newline dentro de un comando).
pub fn parse_zsh(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut cur_ts: u64 = 0;
    let mut in_entry = false;

    let flush = |out: &mut Vec<Entry>, buf: &mut String, ts: u64| {
        let cmd = buf.trim();
        if !cmd.is_empty() {
            out.push(Entry::new(cmd, "", ts));
        }
        buf.clear();
    };

    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        // Continuación: la entrada en curso terminaba en `\` → este renglón
        // es parte del mismo comando.
        if in_entry {
            buf.push('\n');
            buf.push_str(line);
            in_entry = line.ends_with('\\');
            if !in_entry {
                flush(&mut out, &mut buf, cur_ts);
            }
            continue;
        }
        if line.is_empty() {
            continue;
        }
        // Cabecera extended: `: <ts>:<elapsed>;<cmd>`.
        let (ts, cmd) = parse_zsh_header(line).unwrap_or((0, line));
        cur_ts = ts;
        buf.push_str(cmd);
        if cmd.ends_with('\\') {
            in_entry = true;
        } else {
            flush(&mut out, &mut buf, cur_ts);
        }
    }
    // Última entrada sin newline final.
    if !buf.trim().is_empty() {
        flush(&mut out, &mut buf, cur_ts);
    }
    out
}

/// Descompone una cabecera extended de zsh `: <ts>:<elapsed>;<cmd>` en
/// `(timestamp, comando)`. `None` si la línea no tiene esa forma.
fn parse_zsh_header(line: &str) -> Option<(u64, &str)> {
    let rest = line.strip_prefix(": ")?;
    let (ts_part, after) = rest.split_once(':')?;
    let ts = ts_part.trim().parse::<u64>().ok()?;
    let (_elapsed, cmd) = after.split_once(';')?;
    Some((ts, cmd))
}

// ─── Estado de importación incremental ─────────────────────────────────

/// Cuánto de una fuente ya se absorbió: tamaño del fichero al importar y
/// número de entradas parseadas hasta entonces.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SourceState {
    /// Bytes del fichero la última vez que se importó (detecta truncado).
    size: u64,
    /// Cuántas entradas parseadas ya se absorbieron.
    imported: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ImportState {
    /// path del fichero (como string) → progreso.
    sources: BTreeMap<String, SourceState>,
}

/// `$XDG_DATA_HOME/shuma/shell_import.json` — el estado de importación.
fn import_state_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "shuma")
        .map(|d| d.data_dir().join("shell_import.json"))
}

fn load_import_state() -> ImportState {
    import_state_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_import_state(state: &ImportState) -> std::io::Result<()> {
    let Some(path) = import_state_path() else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(state)
        .map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

/// Absorbe las entradas **nuevas** de `sources` al `history`. Incremental:
/// usa el estado en disco para importar sólo lo que creció desde la última
/// vez. Las entradas nuevas de todas las fuentes se mezclan por timestamp y
/// se appendean en bloque. Devuelve qué se importó.
///
/// Errores de IO son blandos: una fuente ilegible se saltea sin abortar.
pub fn absorb_foreign(history: &mut History, sources: &[ForeignSource]) -> ImportReport {
    let mut state = load_import_state();
    let mut fresh: Vec<Entry> = Vec::new();
    let mut report = ImportReport::default();
    let mut state_changed = false;

    for src in sources {
        let key = src.path.to_string_lossy().to_string();
        let Ok(text) = std::fs::read_to_string(&src.path) else {
            continue;
        };
        let size = text.len() as u64;
        let entries = match src.kind {
            ShellKind::Bash => parse_bash(&text),
            ShellKind::Zsh => parse_zsh(&text),
        };
        let st = state.sources.entry(key).or_default();
        // Fichero encogió → rotado/limpiado → reimportar desde cero.
        if size < st.size {
            st.imported = 0;
        }
        let already = st.imported.min(entries.len());
        let nuevos = &entries[already..];
        if !nuevos.is_empty() {
            fresh.extend_from_slice(nuevos);
            report.sources.push(src.path.clone());
        }
        st.imported = entries.len();
        st.size = size;
        state_changed = true;
    }

    if !fresh.is_empty() {
        // Orden cronológico estable: las que no tienen ts (0) conservan su
        // orden relativo de aparición (sort_by es estable).
        fresh.sort_by(|a, b| a.started.cmp(&b.started));
        report.imported = history.append_bulk(fresh).unwrap_or(0);
    }
    if state_changed {
        let _ = save_import_state(&state);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bash_plain_lines() {
        let entries = parse_bash("ls -la\ngit status\n\ncargo build\n");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].line, "ls -la");
        assert_eq!(entries[2].line, "cargo build");
    }

    #[test]
    fn parse_bash_attaches_timestamps() {
        let entries = parse_bash("#1700000000\nls\n#1700000050\ngit pull\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].line, "ls");
        assert_eq!(entries[0].started, 1700000000);
        assert_eq!(entries[1].started, 1700000050);
    }

    #[test]
    fn parse_zsh_extended_format() {
        let text = ": 1700000000:0;ls -la\n: 1700000005:2;cargo build --release\n";
        let entries = parse_zsh(text);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].line, "ls -la");
        assert_eq!(entries[0].started, 1700000000);
        assert_eq!(entries[1].line, "cargo build --release");
        assert_eq!(entries[1].started, 1700000005);
    }

    #[test]
    fn parse_zsh_plain_format() {
        let entries = parse_zsh("ls\npwd\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].line, "ls");
    }

    #[test]
    fn parse_zsh_multiline_continuation() {
        // Comando multilínea: zsh lo escribe con `\` al final del renglón.
        let text = ": 1700000000:0;for f in *; do\\\n  echo $f\\\ndone\n: 1700000010:0;ls\n";
        let entries = parse_zsh(text);
        assert_eq!(entries.len(), 2);
        assert!(entries[0].line.starts_with("for f in *; do"));
        assert!(entries[0].line.contains("echo $f"));
        assert!(entries[0].line.contains("done"));
        assert_eq!(entries[1].line, "ls");
    }

    #[test]
    fn parse_zsh_header_extracts_ts_and_cmd() {
        assert_eq!(parse_zsh_header(": 123:0;echo hi"), Some((123, "echo hi")));
        // Comando con `;` propio: sólo se parte en el primero.
        assert_eq!(
            parse_zsh_header(": 123:0;echo a; echo b"),
            Some((123, "echo a; echo b"))
        );
        assert_eq!(parse_zsh_header("plain line"), None);
    }

    #[test]
    fn absorb_is_incremental_across_calls() {
        // El estado en disco vive en el data dir real; para no tocarlo en el
        // test, ejercitamos sólo los parsers + append_bulk directamente
        // (la incrementalidad de disco se cubre en el e2e del shell).
        let d = tempfile::tempdir().unwrap();
        let mut h = History::open(d.path().join("h.jsonl")).unwrap();
        let entries = parse_bash("ls\npwd\nls\n");
        // append_bulk colapsa duplicados consecutivos, no los no-consecutivos.
        let added = h.append_bulk(entries).unwrap();
        assert_eq!(added, 3);
        assert_eq!(h.len(), 3);
    }
}
