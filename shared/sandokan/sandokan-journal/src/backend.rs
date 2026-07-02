//! Backend de archivo: el journal como jsonl append-only + snapshot sibling.
//!
//! Una entrada por línea (`JournalEntry` en JSON). Es el formato durable de
//! producción: `append` abre en modo append y escribe una línea; `entries` lee
//! y parsea; la compactación reescribe el archivo vacío y persiste el snapshot
//! en un sibling `.snap.json` (como `nakui-core` hace con su log ↔ snapshot).

use crate::{JournalBackend, JournalEntry, JournalError, Snapshot};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Backend persistente sobre un archivo jsonl.
#[derive(Debug, Clone)]
pub struct FileBackend {
    path: PathBuf,
}

impl FileBackend {
    /// Abre (perezosamente) el journal en `path`. No crea el archivo hasta el
    /// primer `append`; un journal inexistente se lee como vacío.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Ruta del snapshot sibling: `<journal>.snap.json`.
    fn snapshot_path(&self) -> PathBuf {
        self.path.with_extension("snap.json")
    }
}

fn io<E: std::fmt::Display>(e: E) -> JournalError {
    JournalError::Io(e.to_string())
}
fn serde<E: std::fmt::Display>(e: E) -> JournalError {
    JournalError::Serde(e.to_string())
}

fn read_optional(path: &Path) -> Result<Option<String>, JournalError> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(e)),
    }
}

impl JournalBackend for FileBackend {
    fn append(&mut self, entry: &JournalEntry) -> Result<(), JournalError> {
        let line = serde_json::to_string(entry).map_err(serde)?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(io)?;
        writeln!(f, "{line}").map_err(io)?;
        Ok(())
    }

    fn entries(&self) -> Result<Vec<JournalEntry>, JournalError> {
        let Some(contents) = read_optional(&self.path)? else {
            return Ok(Vec::new());
        };
        contents
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).map_err(serde))
            .collect()
    }

    fn replace_all(&mut self, entries: &[JournalEntry]) -> Result<(), JournalError> {
        let mut buf = String::new();
        for e in entries {
            buf.push_str(&serde_json::to_string(e).map_err(serde)?);
            buf.push('\n');
        }
        std::fs::write(&self.path, buf).map_err(io)?;
        Ok(())
    }

    fn store_snapshot(&mut self, snap: &Snapshot) -> Result<(), JournalError> {
        let s = serde_json::to_string(snap).map_err(serde)?;
        std::fs::write(self.snapshot_path(), s).map_err(io)?;
        Ok(())
    }

    fn load_snapshot(&self) -> Result<Option<Snapshot>, JournalError> {
        match read_optional(&self.snapshot_path())? {
            Some(s) => Ok(Some(serde_json::from_str(&s).map_err(serde)?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Journal, JournalRecord};
    use sandokan_core::LifecycleEvent;
    use sandokan_lifecycle::LifecycleState;
    use ulid::Ulid;

    /// Ruta temporal única (evita colisión entre tests/jobs paralelos). Sin
    /// `Date::now`/`rand`: usamos un `Ulid` (monótono por proceso) como nombre.
    struct TempJournal(PathBuf);
    impl TempJournal {
        fn new() -> Self {
            let mut p = std::env::temp_dir();
            p.push(format!("sandokan-journal-test-{}.jsonl", Ulid::new()));
            TempJournal(p)
        }
        fn backend(&self) -> FileBackend {
            FileBackend::open(self.0.clone())
        }
    }
    impl Drop for TempJournal {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(self.0.with_extension("snap.json"));
        }
    }

    fn spawned(id: Ulid, pid: i32) -> JournalRecord {
        JournalRecord::Lifecycle(LifecycleEvent::Spawned {
            card_id: id,
            pid: Some(pid),
        })
    }
    fn changed(id: Ulid, state: LifecycleState) -> JournalRecord {
        JournalRecord::Lifecycle(LifecycleEvent::StateChanged {
            card_id: id,
            state,
        })
    }

    #[test]
    fn journal_inexistente_se_lee_vacio() {
        let tmp = TempJournal::new();
        let j = Journal::open(tmp.backend()).unwrap();
        assert_eq!(j.next_seq(), 1);
        assert_eq!(j.state().units.len(), 0);
    }

    #[test]
    fn persiste_y_reabre_desde_disco() {
        let tmp = TempJournal::new();
        let id = Ulid::new();
        {
            let mut j = Journal::open(tmp.backend()).unwrap();
            j.record_intent(id, "worker").unwrap();
            j.record(spawned(id, 100)).unwrap();
            j.record(changed(id, LifecycleState::Running)).unwrap();
        } // se "cae" el proceso; el archivo queda en disco

        let j = Journal::open(tmp.backend()).unwrap();
        let u = j.state().get(&id).unwrap();
        assert_eq!(u.state, LifecycleState::Running);
        assert_eq!(u.pid, Some(100));
        assert_eq!(u.label.as_deref(), Some("worker"));
        assert_eq!(j.next_seq(), 4);
    }

    #[test]
    fn compactar_en_disco_deja_snapshot_y_log_vacio() {
        let tmp = TempJournal::new();
        let id = Ulid::new();
        {
            let mut j = Journal::open(tmp.backend()).unwrap();
            j.record(spawned(id, 5)).unwrap();
            j.record(changed(id, LifecycleState::Running)).unwrap();
            j.compact().unwrap();
        }
        // El log jsonl quedó vacío; el snapshot sibling tiene el estado.
        assert!(tmp.backend().entries().unwrap().is_empty());
        let j = Journal::open(tmp.backend()).unwrap();
        assert_eq!(
            j.state().get(&id).unwrap().state,
            LifecycleState::Running
        );
        assert_eq!(j.next_seq(), 3);
    }
}
