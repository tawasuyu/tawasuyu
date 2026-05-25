//! `shuma-history` — historial **durable** de comandos.
//!
//! Independiente del historial vivo de [`shuma_session::WorkSession`]
//! (que guarda salida completa para la vista en curso): aquí sólo se
//! persisten *líneas* con su contexto mínimo, en un fichero JSONL
//! append‑only fácil de leer, rotar y compartir entre sesiones.
//!
//! Diseño:
//!
//! - **JSONL** (`{"line":...,"cwd":...,"exit":...,"started":...,"duration_ms":...}`).
//!   Una entrada por línea, append‑only — robusto frente a kills/crashes.
//! - **Sin lock global**: las escrituras usan `OpenOptions::append`, que
//!   en Linux son atómicas hasta `PIPE_BUF` (4096 B). Las líneas largas
//!   no se entrelazan en la práctica con los tamaños típicos de un
//!   comando.
//! - **Búsqueda fuzzy** con [`nucleo_matcher`] — mismo matcher que
//!   helix‑editor: rápido, Unicode‑correct, ranking estable.
//! - **Dedup**: política configurable; por defecto se ignora el
//!   duplicado *consecutivo* (estilo bash `HISTCONTROL=ignoredups`).

#![forbid(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Una entrada del historial durable — la línea y su contexto mínimo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// La línea de comandos tal como se ejecutó.
    pub line: String,
    /// Directorio en que se lanzó.
    pub cwd: String,
    /// Código de salida (`None` si nunca terminó —p. ej. crash del shell).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    /// Segundo Unix en que arrancó.
    pub started: u64,
    /// Duración en milisegundos (`None` si no terminó).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl Entry {
    /// Construye una entrada nueva con la línea y el cwd; resto a vacío.
    pub fn new(line: impl Into<String>, cwd: impl Into<String>, started: u64) -> Self {
        Self {
            line: line.into(),
            cwd: cwd.into(),
            exit: None,
            started,
            duration_ms: None,
        }
    }
}

/// Política de deduplicación al añadir entradas nuevas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupPolicy {
    /// Guardar todas las entradas sin deduplicar.
    None,
    /// Saltar el comando si es idéntico al último guardado (`ignoredups`).
    IgnoreConsecutive,
    /// Borrar duplicados previos cuando se vuelve a ver el mismo comando.
    EraseDups,
}

impl Default for DedupPolicy {
    fn default() -> Self {
        Self::IgnoreConsecutive
    }
}

/// Dirección de navegación por el historial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    /// Hacia atrás en el tiempo (flecha arriba).
    Older,
    /// Hacia adelante en el tiempo (flecha abajo).
    Newer,
}

/// Historial durable cargado en memoria, con su fichero de respaldo.
pub struct History {
    path: PathBuf,
    entries: Vec<Entry>,
    dedup: DedupPolicy,
    /// Cuántas líneas inválidas se descartaron al cargar.
    skipped: usize,
}

impl History {
    /// Ruta por defecto: `$XDG_DATA_HOME/shuma/history.jsonl` (o el
    /// equivalente Linux/macOS/Windows según [`directories`]). `None` si
    /// el SO no expone un directorio de datos para el usuario.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "shuma")
            .map(|d| d.data_dir().join("history.jsonl"))
    }

    /// Abre (o crea) el historial en `path`. Carga todas las entradas
    /// existentes. Las líneas inválidas se cuentan en `skipped` pero no
    /// abortan la apertura — el shell debe poder arrancar incluso con
    /// historial parcialmente corrupto.
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut entries = Vec::new();
        let mut skipped = 0usize;
        if path.exists() {
            let f = File::open(&path)?;
            for line in BufReader::new(f).lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Entry>(&line) {
                    Ok(e) => entries.push(e),
                    Err(_) => skipped += 1,
                }
            }
        }
        Ok(Self { path, entries, dedup: DedupPolicy::default(), skipped })
    }

    /// Política de deduplicación activa.
    pub fn dedup(&self) -> DedupPolicy {
        self.dedup
    }

    /// Cambia la política de deduplicación.
    pub fn set_dedup(&mut self, policy: DedupPolicy) {
        self.dedup = policy;
    }

    /// Líneas inválidas descartadas en la última apertura.
    pub fn skipped_on_load(&self) -> usize {
        self.skipped
    }

    /// Cantidad de entradas en memoria.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` si no hay entradas.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Ruta del fichero de respaldo.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Entradas en orden cronológico (más antigua primero).
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Última entrada.
    pub fn last(&self) -> Option<&Entry> {
        self.entries.last()
    }

    /// Añade una entrada — aplica la política de dedup y persiste a
    /// disco. Devuelve `true` si efectivamente se añadió (no era un
    /// duplicado descartable). Las entradas con `line` vacía se ignoran.
    pub fn append(&mut self, entry: Entry) -> io::Result<bool> {
        if entry.line.trim().is_empty() {
            return Ok(false);
        }
        match self.dedup {
            DedupPolicy::None => {}
            DedupPolicy::IgnoreConsecutive => {
                if self.entries.last().is_some_and(|e| e.line == entry.line) {
                    return Ok(false);
                }
            }
            DedupPolicy::EraseDups => {
                self.entries.retain(|e| e.line != entry.line);
                self.rewrite_file()?;
            }
        }
        self.write_one(&entry)?;
        self.entries.push(entry);
        Ok(true)
    }

    /// Actualiza la última entrada con el código de salida y la duración
    /// cuando el comando termina. Persiste reescribiendo el fichero.
    pub fn finalize_last(&mut self, exit: i32, duration_ms: u64) -> io::Result<()> {
        if let Some(last) = self.entries.last_mut() {
            last.exit = Some(exit);
            last.duration_ms = Some(duration_ms);
            self.rewrite_file()?;
        }
        Ok(())
    }

    /// Navegación por el historial — devuelve el `(index, entry)`
    /// correspondiente a moverse `dir` desde el cursor actual. El cursor
    /// `None` parte "del final" (por debajo de la última entrada).
    /// Convención: el índice 0 es la **entrada más reciente**, y avanza
    /// hacia el pasado al subir el cursor.
    pub fn navigate(&self, cursor: Option<usize>, dir: Nav) -> Option<(usize, &Entry)> {
        if self.entries.is_empty() {
            return None;
        }
        let next = match (cursor, dir) {
            (None, Nav::Older) => 0,
            (None, Nav::Newer) => return None,
            (Some(i), Nav::Older) => i + 1,
            (Some(0), Nav::Newer) => return None,
            (Some(i), Nav::Newer) => i - 1,
        };
        if next >= self.entries.len() {
            return None;
        }
        let entry = &self.entries[self.entries.len() - 1 - next];
        Some((next, entry))
    }

    /// Búsqueda fuzzy sobre el campo `line`. Devuelve hasta `limit`
    /// resultados ordenados por score descendente. Una `query` vacía
    /// devuelve las entradas más recientes.
    pub fn fuzzy_search(&self, query: &str, limit: usize) -> Vec<&Entry> {
        if limit == 0 || self.entries.is_empty() {
            return Vec::new();
        }
        if query.trim().is_empty() {
            return self.entries.iter().rev().take(limit).collect();
        }
        use nucleo_matcher::{
            pattern::{CaseMatching, Normalization, Pattern},
            Config, Matcher,
        };
        let mut matcher = Matcher::new(Config::DEFAULT);
        let pat = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut scored: Vec<(u32, usize)> = Vec::new();
        let mut buf = Vec::new();
        for (idx, e) in self.entries.iter().enumerate() {
            buf.clear();
            let hay = nucleo_matcher::Utf32Str::new(&e.line, &mut buf);
            if let Some(score) = pat.score(hay, &mut matcher) {
                scored.push((score, idx));
            }
        }
        // Score desc, y a igualdad de score, el más reciente primero.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        scored
            .into_iter()
            .take(limit)
            .map(|(_, i)| &self.entries[i])
            .collect()
    }

    // --- I/O ---

    fn write_one(&self, entry: &Entry) -> io::Result<()> {
        let mut f = OpenOptions::new().create(true).append(true).open(&self.path)?;
        let mut s = serde_json::to_string(entry).map_err(io::Error::other)?;
        s.push('\n');
        f.write_all(s.as_bytes())?;
        f.flush()
    }

    fn rewrite_file(&self) -> io::Result<()> {
        // Escritura atómica vía rename — nunca dejamos un historial a medias.
        let tmp = self.path.with_extension("jsonl.tmp");
        {
            let mut f = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&tmp)?;
            for e in &self.entries {
                let mut s = serde_json::to_string(e).map_err(io::Error::other)?;
                s.push('\n');
                f.write_all(s.as_bytes())?;
            }
            f.flush()?;
        }
        std::fs::rename(tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn h(dir: &Path) -> History {
        History::open(dir.join("history.jsonl")).unwrap()
    }

    #[test]
    fn empty_history_round_trip() {
        let d = tempdir().unwrap();
        let h1 = h(d.path());
        assert!(h1.is_empty());
        let h2 = h(d.path());
        assert!(h2.is_empty());
    }

    #[test]
    fn append_persists_across_reopen() {
        let d = tempdir().unwrap();
        {
            let mut h = h(d.path());
            h.append(Entry::new("ls", "/tmp", 1000)).unwrap();
            h.append(Entry::new("pwd", "/tmp", 1001)).unwrap();
        }
        let h = h(d.path());
        assert_eq!(h.len(), 2);
        assert_eq!(h.entries()[0].line, "ls");
        assert_eq!(h.entries()[1].line, "pwd");
    }

    #[test]
    fn ignore_consecutive_dedup_skips_repeats() {
        let d = tempdir().unwrap();
        let mut h = h(d.path());
        assert!(h.append(Entry::new("ls", "/tmp", 1)).unwrap());
        assert!(!h.append(Entry::new("ls", "/tmp", 2)).unwrap());
        assert!(h.append(Entry::new("pwd", "/tmp", 3)).unwrap());
        assert!(h.append(Entry::new("ls", "/tmp", 4)).unwrap());
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn erase_dups_purges_prior_copies() {
        let d = tempdir().unwrap();
        let mut h = h(d.path());
        h.set_dedup(DedupPolicy::EraseDups);
        h.append(Entry::new("ls", "/tmp", 1)).unwrap();
        h.append(Entry::new("pwd", "/tmp", 2)).unwrap();
        h.append(Entry::new("ls", "/tmp", 3)).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(h.entries()[0].line, "pwd");
        assert_eq!(h.entries()[1].line, "ls");
    }

    #[test]
    fn empty_line_is_ignored() {
        let d = tempdir().unwrap();
        let mut h = h(d.path());
        assert!(!h.append(Entry::new("", "/tmp", 1)).unwrap());
        assert!(!h.append(Entry::new("   ", "/tmp", 2)).unwrap());
        assert!(h.is_empty());
    }

    #[test]
    fn finalize_writes_exit_and_duration() {
        let d = tempdir().unwrap();
        {
            let mut h = h(d.path());
            h.append(Entry::new("sleep 1", "/tmp", 0)).unwrap();
            h.finalize_last(0, 1000).unwrap();
        }
        let h = h(d.path());
        assert_eq!(h.last().unwrap().exit, Some(0));
        assert_eq!(h.last().unwrap().duration_ms, Some(1000));
    }

    #[test]
    fn navigate_walks_from_newest_to_oldest() {
        let d = tempdir().unwrap();
        let mut h = h(d.path());
        for (i, l) in ["a", "b", "c"].iter().enumerate() {
            h.append(Entry::new(*l, "/tmp", i as u64)).unwrap();
        }
        // Empezando sin cursor, Older da la más reciente.
        let (i0, e0) = h.navigate(None, Nav::Older).unwrap();
        assert_eq!((i0, e0.line.as_str()), (0, "c"));
        let (i1, e1) = h.navigate(Some(i0), Nav::Older).unwrap();
        assert_eq!((i1, e1.line.as_str()), (1, "b"));
        let (i2, e2) = h.navigate(Some(i1), Nav::Older).unwrap();
        assert_eq!((i2, e2.line.as_str()), (2, "a"));
        // En el extremo no hay más viejas.
        assert!(h.navigate(Some(i2), Nav::Older).is_none());
        // Volvemos hacia las nuevas.
        let (i3, e3) = h.navigate(Some(i2), Nav::Newer).unwrap();
        assert_eq!((i3, e3.line.as_str()), (1, "b"));
    }

    #[test]
    fn fuzzy_search_ranks_matches() {
        let d = tempdir().unwrap();
        let mut h = h(d.path());
        for l in ["cargo build --release", "cargo test", "git status", "cargo run"] {
            h.append(Entry::new(l, "/tmp", 0)).unwrap();
        }
        let hits = h.fuzzy_search("cgo", 10);
        // Las 3 entradas con "cargo" matchean; "git status" no.
        assert_eq!(hits.len(), 3);
        assert!(hits.iter().all(|e| e.line.contains("cargo")));
    }

    #[test]
    fn empty_query_returns_most_recent_first() {
        let d = tempdir().unwrap();
        let mut h = h(d.path());
        for l in ["a", "b", "c", "d"] {
            h.append(Entry::new(l, "/tmp", 0)).unwrap();
        }
        let hits = h.fuzzy_search("", 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].line, "d");
        assert_eq!(hits[1].line, "c");
    }

    #[test]
    fn corrupt_lines_are_skipped_not_fatal() {
        let d = tempdir().unwrap();
        let path = d.path().join("history.jsonl");
        std::fs::write(
            &path,
            "{\"line\":\"ok\",\"cwd\":\"/tmp\",\"started\":1}\ngarbage\n{\"line\":\"ok2\",\"cwd\":\"/tmp\",\"started\":2}\n",
        )
        .unwrap();
        let h = History::open(&path).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(h.skipped_on_load(), 1);
    }
}
