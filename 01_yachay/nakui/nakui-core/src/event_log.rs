//! Append-only event log for deterministic replay.
//!
//! Two entry kinds:
//!   - `Seed`: an externally-provided initial record (the system boundary).
//!   - `Morphism`: a successful kernel-validated morphism call, with the
//!     produced ops attached.
//!
//! `replay()` reconstructs a store by reading the log and applying ops
//! directly — fast, no script execution. `verify_log()` re-runs every
//! morphism through the kernel and asserts the recomputed ops match the
//! logged ones, which is the operational definition of determinism.
//!
//! Failures are never logged: a rejected morphism produces no event.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

use crate::delta::FieldOp;
use crate::executor::{ExecError, Executor};
use crate::store::{MemoryStore, Store, StoreError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogEntry {
    Seed {
        seq: u64,
        entity: String,
        id: Uuid,
        data: Value,
        /// Bundle hash (just the KCL schemas) at the moment this seed
        /// was logged. `None` for pre-versioning entries — `verify_log`
        /// skips the schema check on those. New writes always populate
        /// it via `seed_and_log`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema_hash: Option<[u8; 32]>,
    },
    Morphism {
        seq: u64,
        morphism: String,
        inputs: BTreeMap<String, Uuid>,
        params: Value,
        ops: Vec<FieldOp>,
        /// Hash of (kcl bundle | manifest spec | rhai script bytes) at
        /// the moment this event was logged. `None` for pre-versioning
        /// entries — `verify_log` skips the schema check on those (they
        /// predate the contract). New writes always populate it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema_hash: Option<[u8; 32]>,
    },
}

impl LogEntry {
    pub fn seq(&self) -> u64 {
        match self {
            LogEntry::Seed { seq, .. } => *seq,
            LogEntry::Morphism { seq, .. } => *seq,
        }
    }
}

#[derive(Debug, Error)]
pub enum LogError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse at line {line}: {source}")]
    Parse {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("non-monotonic seq: got {got}, expected {expected}")]
    NonMonotonic { got: u64, expected: u64 },
}

/// Errors from `execute_and_log`. The variants distinguish *when in the
/// pipeline* the failure occurred — which determines whether the log was
/// updated and whether the live store is still consistent.
#[derive(Debug, Error)]
pub enum ExecuteError {
    /// Failure before the log was written. Store untouched, log untouched.
    /// Safe to retry with the same inputs.
    #[error("pre-log validation failed: {0}")]
    PreLog(#[from] ExecError),

    /// Log append failed (typically IO). Store untouched, log untouched.
    /// Safe to retry once the log backend recovers.
    #[error("log append failed: {0}")]
    LogAppend(#[from] LogError),

    /// Apply to the store failed AFTER the event was logged. The log is
    /// canonical; the live store is now stale and should be rebuilt by
    /// replaying the log. Retrying the same morphism is incorrect — the
    /// event is already on disk.
    #[error("store apply failed after log was committed (log is canonical, store stale): {0}")]
    PostLogStore(crate::store::StoreError),
}

#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("log: {0}")]
    Log(#[from] LogError),
    #[error("store: {0}")]
    Store(#[from] StoreError),
}

/// A reconcile rebuilds a stale store from the log. Either the wipe step
/// or the replay step can fail.
#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error("clearing store before replay failed: {0}")]
    Clear(#[source] StoreError),
    #[error("replay into cleared store failed: {0}")]
    Replay(#[from] ReplayError),
}

/// Outcome of `execute_and_log_with_recovery`. PreLog/LogAppend mirror the
/// pre-WAL-fence variants of `ExecuteError` — the store is untouched and
/// the caller can retry. `Unrecoverable` means the WAL fence was crossed
/// (event is canonical on disk) but reconcile *also* failed: the operator
/// must intervene before any further writes.
#[derive(Debug, Error)]
pub enum RecoverableExecuteError {
    #[error("pre-log validation failed: {0}")]
    PreLog(#[from] ExecError),
    #[error("log append failed: {0}")]
    LogAppend(#[from] LogError),
    #[error(
        "store apply failed AND reconcile failed — log is canonical, store is in an unknown state. apply: {post_log}; reconcile: {reconcile}"
    )]
    Unrecoverable {
        #[source]
        post_log: StoreError,
        reconcile: ReconcileError,
    },
}

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("log: {0}")]
    Log(#[from] LogError),
    #[error("morphism replay failed at seq {seq}: {source}")]
    Exec {
        seq: u64,
        #[source]
        source: ExecError,
    },
    #[error(
        "non-determinism at seq {seq} morphism `{morphism}`: recomputed ops differ from logged ops"
    )]
    OpsMismatch {
        seq: u64,
        morphism: String,
        expected: Vec<FieldOp>,
        actual: Vec<FieldOp>,
    },
    /// The morphism was logged under a different schema/script bundle
    /// than the one currently loaded. Re-executing it would (likely)
    /// produce different ops, but the more specific signal is "the
    /// rules changed since this was logged" — actionable: migrate the
    /// log, or pin the executor to a compatible version.
    #[error(
        "schema mismatch at seq {seq} morphism `{morphism}`: logged schema_hash differs from current executor"
    )]
    SchemaMismatch {
        seq: u64,
        morphism: String,
        logged: [u8; 32],
        current: [u8; 32],
    },
    /// A `Seed` entry was logged under a different KCL bundle than the
    /// one currently loaded. The seed's data may no longer fit the
    /// entity definition. Coarser than `SchemaMismatch` (any change
    /// to any schema file flips it, even one that doesn't affect the
    /// seeded entity) but the operator still wants to know.
    #[error(
        "seed schema mismatch at seq {seq} entity `{entity}` id {id}: logged bundle hash differs from current executor"
    )]
    SeedSchemaMismatch {
        seq: u64,
        entity: String,
        id: Uuid,
        logged: [u8; 32],
        current: [u8; 32],
    },
}

pub struct EventLog {
    path: PathBuf,
    next_seq: u64,
}

impl EventLog {
    /// Open or create a log at `path`. Reads existing entries to compute
    /// `next_seq` and validate monotonicity. The first entry can start at
    /// any seq (compacted logs are rooted at seq > 0); subsequent entries
    /// must be strictly contiguous.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, LogError> {
        let path = path.into();
        let mut next_seq: u64 = 0;
        if path.exists() {
            let entries = read_entries(&path)?;
            let mut iter = entries.iter();
            if let Some(first) = iter.next() {
                next_seq = first.seq() + 1;
                for e in iter {
                    if e.seq() != next_seq {
                        return Err(LogError::NonMonotonic {
                            got: e.seq(),
                            expected: next_seq,
                        });
                    }
                    next_seq = e.seq() + 1;
                }
            }
        }
        Ok(Self { path, next_seq })
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append an entry. Calls `sync_all()` so the entry is durable on disk
    /// before returning Ok — this is the WAL fence: by the time the caller
    /// proceeds to mutate the store, the event is recoverable from a power
    /// loss.
    pub fn append(&mut self, entry: LogEntry) -> Result<(), LogError> {
        if entry.seq() != self.next_seq {
            return Err(LogError::NonMonotonic {
                got: entry.seq(),
                expected: self.next_seq,
            });
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let s = serde_json::to_string(&entry).expect("LogEntry serializes");
        f.write_all(s.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        self.next_seq += 1;
        Ok(())
    }

    pub fn entries(&self) -> Result<Vec<LogEntry>, LogError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        read_entries(&self.path)
    }

    /// Truncate the log to drop entries with `seq <= through_seq`.
    /// IRREVERSIBLE: caller must verify a Snapshot covering `through_seq`
    /// exists on durable storage before calling this — once the entries
    /// are gone, replay can only start from the snapshot.
    ///
    /// Atomic at the filesystem level: writes survivors to a sibling
    /// tempfile then renames over the original.
    pub fn compact_through(&mut self, through_seq: u64) -> Result<(), LogError> {
        let survivors: Vec<LogEntry> = self
            .entries()?
            .into_iter()
            .filter(|e| e.seq() > through_seq)
            .collect();

        let tmp = self.path.with_extension("compacting");
        {
            let mut f = std::fs::File::create(&tmp)?;
            for e in &survivors {
                let s = serde_json::to_string(e).expect("LogEntry serializes");
                f.write_all(s.as_bytes())?;
                f.write_all(b"\n")?;
            }
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        sync_parent_dir(&self.path)?;
        Ok(())
    }
}

/// Open and fsync the parent directory of `target`. After an atomic
/// rename, the directory entry change isn't durable until the directory
/// itself is fsynced — without this, a kernel/power crash between the
/// rename and the next disk flush could leave the directory in a state
/// where the rename never happened (depending on filesystem journal
/// mode). With it, the rename survives.
///
/// Best-effort on platforms where opening a directory for sync isn't
/// permitted: the syscalls are POSIX-portable across Linux, macOS, and
/// the BSDs (the OSes Nakui targets), so this generally succeeds. A
/// failure here is propagated as an IO error so the caller can choose
/// to surface it; we prefer "loud" over "silent" for durability code.
fn sync_parent_dir(target: &Path) -> std::io::Result<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let dir = std::fs::File::open(parent)?;
    dir.sync_all()
}

/// A snapshot of a `Store`'s state at a particular log seq. Lets us short-
/// circuit replay: load the snapshot, then apply only the events with
/// `seq > snapshot.seq`. MemoryStore-specific for V1 — backends that
/// already persist (SurrealStore + RocksDB) don't need this layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// The last log seq this snapshot subsumes. `replay` resumes at seq+1.
    pub seq: u64,
    /// Full state at that seq, in MemoryStore's native shape.
    pub records: HashMap<String, HashMap<Uuid, Value>>,
    /// Module schema hash at capture time. `Some` for snapshots taken
    /// via `capture(_, _, executor)`; `None` for those taken via the
    /// hash-unaware `from_memory_store`. Loaders use this to refuse a
    /// snapshot produced under a different bundle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_hash: Option<[u8; 32]>,
}

#[derive(Debug, Error)]
pub enum SnapshotMismatchError {
    #[error(
        "snapshot schema_hash differs from current executor; refusing to load (snapshot was taken under a different module bundle)"
    )]
    SchemaMismatch {
        snapshot: [u8; 32],
        current: [u8; 32],
    },
}

impl Snapshot {
    /// Capture the in-memory store's current state without binding to a
    /// schema bundle. Test fixtures and ad-hoc tooling call this; the
    /// production path uses `capture` so the snapshot can be validated
    /// against the executor on load.
    pub fn from_memory_store(store: &MemoryStore, seq: u64) -> Self {
        Self {
            seq,
            records: store.records().clone(),
            schema_hash: None,
        }
    }

    /// Production capture: stamp the snapshot with the executor's
    /// `module_schema_hash` so future loads can refuse a mismatch.
    pub fn capture(store: &MemoryStore, seq: u64, executor: &Executor) -> Self {
        Self {
            seq,
            records: store.records().clone(),
            schema_hash: Some(executor.module_schema_hash()),
        }
    }

    /// Verify the snapshot was produced under a bundle compatible with
    /// `executor`. Snapshots without a hash (legacy / `from_memory_store`)
    /// pass — the operator opted out of this check at capture time.
    pub fn ensure_compatible_with(
        &self,
        executor: &Executor,
    ) -> Result<(), SnapshotMismatchError> {
        let Some(snap_hash) = self.schema_hash else {
            return Ok(());
        };
        let current = executor.module_schema_hash();
        if snap_hash != current {
            return Err(SnapshotMismatchError::SchemaMismatch {
                snapshot: snap_hash,
                current,
            });
        }
        Ok(())
    }

    /// Atomically write the snapshot to `path`. Writes the bytes to a
    /// sibling tempfile (`<path>.writing`), fsyncs, renames over the
    /// target, then fsyncs the parent directory so the rename survives
    /// a crash. A crash mid-write leaves either the previous snapshot
    /// at `path` (rename never happened) or the new one (rename
    /// completed and was durable) — never a truncated file. A stale
    /// tempfile from a prior crash gets overwritten by `File::create`
    /// on the next attempt, so writes are also self-healing.
    pub fn write(&self, path: &Path) -> Result<(), LogError> {
        let data = serde_json::to_vec_pretty(self).expect("snapshot serializes");
        let tmp = path.with_extension("writing");
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&data)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)?;
        sync_parent_dir(path).map_err(LogError::Io)
    }

    pub fn load(path: &Path) -> Result<Option<Self>, LogError> {
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(path).map_err(LogError::Io)?;
        let snap: Snapshot = serde_json::from_str(&text).map_err(|e| LogError::Parse {
            line: 0,
            source: e,
        })?;
        Ok(Some(snap))
    }
}

fn read_entries(path: &Path) -> Result<Vec<LogEntry>, LogError> {
    let f = std::fs::File::open(path)?;
    let r = BufReader::new(f);
    let mut out = Vec::new();
    for (i, line) in r.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: LogEntry = serde_json::from_str(&line).map_err(|e| LogError::Parse {
            line: i + 1,
            source: e,
        })?;
        out.push(entry);
    }
    Ok(out)
}

/// Seed an entity into the store and persist the event.
///
/// WAL order: append to log *first*, then mutate the store. If the log
/// append fails, the store is untouched and the caller can safely retry.
/// `Store::seed` is infallible by trait contract — once the log entry is
/// durable the store update is guaranteed to land for in-memory backends.
/// For backends with fallible writes (network/disk), failures surface as
/// a panic during `seed()`; callers that need a fallible seed path should
/// wrap their own retry/reconcile loop.
pub fn seed_and_log<S: Store>(
    executor: &Executor,
    store: &mut S,
    log: &mut EventLog,
    entity: &str,
    id: Uuid,
    data: Value,
) -> Result<(), LogError> {
    let seq = log.next_seq();
    log.append(LogEntry::Seed {
        seq,
        entity: entity.to_string(),
        id,
        data: data.clone(),
        schema_hash: Some(executor.schema_bundle_hash),
    })?;
    store.seed(entity, id, data);
    // Best-effort: a failure here means next startup does an extra full
    // replay, never a correctness issue.
    let _ = store.set_last_applied_seq(seq);
    Ok(())
}

/// Run a morphism and persist the event in WAL order:
///   1. compute() — pure, no mutation; full kernel validation incl. dry-run.
///   2. log.append() — event hits disk *before* the store changes.
///   3. store.apply() — materialize the change. By WAL semantics the log
///      is now the source of truth: if (3) fails, the stale store can be
///      rebuilt by replaying the log.
///
/// The error variants tell the caller exactly which stage failed so they
/// know whether to retry, recover, or rebuild.
pub fn execute_and_log<S: Store>(
    executor: &Executor,
    store: &mut S,
    log: &mut EventLog,
    morphism: &str,
    inputs: &[(&str, Uuid)],
    params: Value,
) -> Result<Vec<FieldOp>, ExecuteError> {
    let ops = executor.compute(store, morphism, inputs, params.clone())?;
    let seq = log.next_seq();
    let entry = LogEntry::Morphism {
        seq,
        morphism: morphism.to_string(),
        inputs: inputs
            .iter()
            .map(|(r, id)| (r.to_string(), *id))
            .collect(),
        params,
        ops: ops.clone(),
        schema_hash: executor.schema_hash(morphism),
    };
    log.append(entry)?;
    store.apply(&ops).map_err(ExecuteError::PostLogStore)?;
    let _ = store.set_last_applied_seq(seq);
    Ok(ops)
}

/// Rebuild a (possibly stale) store from the log. Wipes the store, then
/// replays every event. Use this after a `PostLogStore` failure: the WAL
/// fence guarantees the log is the source of truth, so a clean replay
/// brings the store back into agreement with it.
///
/// `execute_and_log_with_recovery` automates this for the common case;
/// reach for `reconcile` directly when an operator/CLI is doing the
/// recovery, or when a backend reports drift detected out-of-band.
pub fn reconcile<S: Store>(store: &mut S, log: &EventLog) -> Result<(), ReconcileError> {
    store.clear().map_err(ReconcileError::Clear)?;
    replay_into(log, store)?;
    Ok(())
}

/// Like `execute_and_log`, but on `PostLogStore` automatically rebuilds
/// the store from the log and returns the ops as if the apply had
/// succeeded. The caller sees a consistent post-state — either the event
/// landed cleanly, or it landed via reconcile, or `Unrecoverable` (which
/// means even the rebuild failed and the store must not be trusted).
///
/// PreLog and LogAppend are forwarded verbatim: the WAL fence wasn't
/// crossed, so there's nothing to reconcile.
pub fn execute_and_log_with_recovery<S: Store>(
    executor: &Executor,
    store: &mut S,
    log: &mut EventLog,
    morphism: &str,
    inputs: &[(&str, Uuid)],
    params: Value,
) -> Result<Vec<FieldOp>, RecoverableExecuteError> {
    let ops = executor.compute(store, morphism, inputs, params.clone())?;
    let seq = log.next_seq();
    let entry = LogEntry::Morphism {
        seq,
        morphism: morphism.to_string(),
        inputs: inputs
            .iter()
            .map(|(r, id)| (r.to_string(), *id))
            .collect(),
        params,
        ops: ops.clone(),
        schema_hash: executor.schema_hash(morphism),
    };
    log.append(entry)?;
    if let Err(post_log) = store.apply(&ops) {
        if let Err(reconcile) = reconcile(store, log) {
            return Err(RecoverableExecuteError::Unrecoverable {
                post_log,
                reconcile,
            });
        }
        // After reconcile the store reflects log state up to log.next_seq()-1
        // (which equals our seq). The reconcile path itself updated the
        // marker; nothing more to do here.
    } else {
        let _ = store.set_last_applied_seq(seq);
    }
    Ok(ops)
}

/// Replay the log into a caller-provided `Store`. The store should be
/// empty on entry; existing records are not erased. Use this with any
/// `Store` impl (MemoryStore, SurrealStore, future backends).
pub fn replay_into<S: Store>(log: &EventLog, store: &mut S) -> Result<(), ReplayError> {
    replay_with_snapshot_into(log, None, store)
}

/// Replay starting from a snapshot. If `snapshot` is `Some`, every record
/// in it is seeded into `store` first, then events with `seq > snapshot.seq`
/// are applied. The point: replay cost shrinks from O(events) to
/// O(events_after_snapshot), useful when the log grows large.
pub fn replay_with_snapshot_into<S: Store>(
    log: &EventLog,
    snapshot: Option<&Snapshot>,
    store: &mut S,
) -> Result<(), ReplayError> {
    let start_seq = if let Some(snap) = snapshot {
        for (entity, recs) in &snap.records {
            for (id, data) in recs {
                store.seed(entity, *id, data.clone());
            }
        }
        snap.seq + 1
    } else {
        0
    };

    let mut last_applied: Option<u64> = snapshot.map(|s| s.seq);
    for entry in log.entries()? {
        if entry.seq() < start_seq {
            continue;
        }
        let seq = entry.seq();
        match entry {
            LogEntry::Seed {
                entity, id, data, ..
            } => store.seed(&entity, id, data),
            LogEntry::Morphism { ops, .. } => store.apply(&ops)?,
        }
        last_applied = Some(seq);
    }
    if let Some(seq) = last_applied {
        let _ = store.set_last_applied_seq(seq);
    }
    Ok(())
}

/// Convenience: replay into a fresh `MemoryStore`. The fast path: O(events)
/// with no Rhai execution.
pub fn replay(log: &EventLog) -> Result<MemoryStore, ReplayError> {
    let mut store = MemoryStore::new();
    replay_into(log, &mut store)?;
    Ok(store)
}

/// Re-execute every logged morphism through the kernel and assert the
/// recomputed ops match the logged ops byte-for-byte. This is the
/// determinism contract: if it ever fails, a morphism became impure.
pub fn verify_log(log: &EventLog, executor: &Executor) -> Result<(), VerifyError> {
    let mut store = MemoryStore::new();
    for entry in log.entries()? {
        match entry {
            LogEntry::Seed {
                seq,
                entity,
                id,
                data,
                schema_hash,
            } => {
                if let Some(logged_hash) = schema_hash {
                    let current_hash = executor.schema_bundle_hash;
                    if logged_hash != current_hash {
                        return Err(VerifyError::SeedSchemaMismatch {
                            seq,
                            entity,
                            id,
                            logged: logged_hash,
                            current: current_hash,
                        });
                    }
                }
                store.seed(&entity, id, data);
            }
            LogEntry::Morphism {
                seq,
                morphism,
                inputs,
                params,
                ops: logged,
                schema_hash,
            } => {
                // Schema check first: if the rules changed, re-execution
                // is meaningless — it'd just surface as OpsMismatch with
                // a less actionable message. Legacy entries with no
                // hash predate the contract; we let those through.
                if let Some(logged_hash) = schema_hash {
                    if let Some(current_hash) = executor.schema_hash(&morphism) {
                        if logged_hash != current_hash {
                            return Err(VerifyError::SchemaMismatch {
                                seq,
                                morphism,
                                logged: logged_hash,
                                current: current_hash,
                            });
                        }
                    }
                }
                let owned: Vec<(String, Uuid)> = inputs.into_iter().collect();
                let refs: Vec<(&str, Uuid)> =
                    owned.iter().map(|(r, id)| (r.as_str(), *id)).collect();
                let recomputed = executor
                    .run(&mut store, &morphism, &refs, params)
                    .map_err(|e| VerifyError::Exec { seq, source: e })?;
                if recomputed != logged {
                    return Err(VerifyError::OpsMismatch {
                        seq,
                        morphism,
                        expected: logged,
                        actual: recomputed,
                    });
                }
            }
        }
    }
    Ok(())
}
