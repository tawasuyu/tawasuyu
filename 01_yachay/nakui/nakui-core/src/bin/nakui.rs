//! `nakui` — operator CLI for inspecting, replaying, and verifying an
//! event log produced by the kernel. The three subcommands map to the
//! three things you need when something goes sideways in production:
//!
//!   - `inspect`     — what's in the log? (audit trail)
//!   - `replay`      — what state does the log produce? (recovery dry-run)
//!   - `verify-log`  — does every morphism still reproduce its ops?
//!                     (determinism contract — the regression alarm)
//!
//! Exit codes: 0 on success, 1 on operational error, 2 on bad arguments.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use nakui_core::drift::{DriftDiff, check_against_socket};
use nakui_core::event_log::{
    EventLog, LogEntry, Snapshot, replay_with_snapshot_into, verify_log,
};
use nakui_core::executor::Executor;
use nakui_core::run::run_server;
use nakui_core::store::MemoryStore;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let prog = args.first().cloned().unwrap_or_else(|| "nakui".into());
    let sub = match args.get(1).map(String::as_str) {
        Some(s) => s,
        None => {
            print_usage(&prog);
            return ExitCode::from(2);
        }
    };
    let rest = &args[2..];

    let result = match sub {
        "inspect" => cmd_inspect(rest),
        "replay" => cmd_replay(rest),
        "verify-log" => cmd_verify_log(rest),
        "run" => cmd_run(rest),
        "drift" => cmd_drift(rest),
        "snapshot" => cmd_snapshot(rest),
        "compact" => cmd_compact(rest),
        "-h" | "--help" | "help" => {
            print_usage(&prog);
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("nakui: unknown subcommand `{}`", other);
            print_usage(&prog);
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::BadArgs(msg)) => {
            eprintln!("nakui: {}", msg);
            print_usage(&prog);
            ExitCode::from(2)
        }
        Err(CliError::Op(msg)) => {
            eprintln!("nakui: {}", msg);
            ExitCode::from(1)
        }
        // Drift uses its own exit code so callers can distinguish "the
        // tool failed" (1) from "the tool worked and detected drift" (3).
        Err(CliError::DriftDetected) => ExitCode::from(3),
    }
}

enum CliError {
    BadArgs(String),
    Op(String),
    DriftDetected,
}

fn print_usage(prog: &str) {
    eprintln!(
        "usage:
  {p} inspect    --log <path>
  {p} replay     --log <path> [--snapshot <path>]
  {p} verify-log --log <path> --module <dir>
  {p} run        --log <path> --module <dir> --socket <path>
                 [--snapshot <path>] [--store-path <dir>]
  {p} drift      --log <path> --against <socket>
  {p} snapshot   --log <path> --module <dir> --out <path>
  {p} compact    --log <path> --snapshot <path>

  --store-path activates persistent SurrealStore (kv-surrealkv);
  requires the binary to be built with `--features persistent`.",
        p = prog
    );
}

/// Minimal flag parser: `--name value` pairs, no `=` form, no clustering.
/// Returns a map of name -> value. Unknown flags are an error so typos
/// surface immediately instead of silently being ignored.
fn parse_flags(args: &[String], allowed: &[&str]) -> Result<BTreeMap<String, String>, CliError> {
    let mut out = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        let flag = &args[i];
        if !flag.starts_with("--") {
            return Err(CliError::BadArgs(format!(
                "expected --flag, got `{}`",
                flag
            )));
        }
        let name = &flag[2..];
        if !allowed.contains(&name) {
            return Err(CliError::BadArgs(format!("unknown flag `--{}`", name)));
        }
        let val = args.get(i + 1).ok_or_else(|| {
            CliError::BadArgs(format!("flag `--{}` requires a value", name))
        })?;
        out.insert(name.to_string(), val.clone());
        i += 2;
    }
    Ok(out)
}

fn require<'a>(
    flags: &'a BTreeMap<String, String>,
    name: &str,
) -> Result<&'a String, CliError> {
    flags
        .get(name)
        .ok_or_else(|| CliError::BadArgs(format!("missing required flag `--{}`", name)))
}

fn cmd_inspect(args: &[String]) -> Result<(), CliError> {
    let flags = parse_flags(args, &["log"])?;
    let log_path = PathBuf::from(require(&flags, "log")?);
    let log = EventLog::open(&log_path).map_err(|e| CliError::Op(format!("open log: {}", e)))?;
    let entries = log
        .entries()
        .map_err(|e| CliError::Op(format!("read log: {}", e)))?;
    println!("log: {}", log.path().display());
    println!("entries: {}", entries.len());
    if entries.is_empty() {
        return Ok(());
    }
    println!("seq range: {}..={}", entries[0].seq(), entries.last().unwrap().seq());
    println!();
    for e in &entries {
        match e {
            LogEntry::Seed {
                seq, entity, id, ..
            } => println!("  #{:04} seed   {} {}", seq, entity, id),
            LogEntry::Morphism {
                seq,
                morphism,
                ops,
                inputs,
                ..
            } => {
                let inputs_s = inputs
                    .iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "  #{:04} morph  {} ({} ops) [{}]",
                    seq,
                    morphism,
                    ops.len(),
                    inputs_s
                );
            }
        }
    }
    Ok(())
}

fn cmd_replay(args: &[String]) -> Result<(), CliError> {
    let flags = parse_flags(args, &["log", "snapshot"])?;
    let log_path = PathBuf::from(require(&flags, "log")?);
    let log = EventLog::open(&log_path).map_err(|e| CliError::Op(format!("open log: {}", e)))?;

    let snapshot = if let Some(p) = flags.get("snapshot") {
        let path = PathBuf::from(p);
        Snapshot::load(&path)
            .map_err(|e| CliError::Op(format!("load snapshot: {}", e)))?
            .ok_or_else(|| CliError::Op(format!("snapshot not found: {}", path.display())))?
            .into()
    } else {
        None::<Snapshot>
    };

    let mut store = MemoryStore::new();
    replay_with_snapshot_into(&log, snapshot.as_ref(), &mut store)
        .map_err(|e| CliError::Op(format!("replay: {}", e)))?;

    let entries = log
        .entries()
        .map_err(|e| CliError::Op(format!("read log: {}", e)))?;
    let last_seq = entries.last().map(|e| e.seq().to_string()).unwrap_or_else(|| "<empty>".into());
    println!("replayed log: {}", log.path().display());
    if let Some(snap) = &snapshot {
        println!("snapshot: seq {} (covers seq <= {})", snap.seq, snap.seq);
    }
    println!("last seq: {}", last_seq);
    println!("entities:");
    let mut by_entity: Vec<(&String, usize)> = store
        .records()
        .iter()
        .map(|(k, v)| (k, v.len()))
        .collect();
    by_entity.sort_by(|a, b| a.0.cmp(b.0));
    if by_entity.is_empty() {
        println!("  (none)");
    } else {
        for (entity, count) in by_entity {
            println!("  {:<20} {}", entity, count);
        }
    }
    Ok(())
}

fn cmd_drift(args: &[String]) -> Result<(), CliError> {
    let flags = parse_flags(args, &["log", "against"])?;
    let log_path = PathBuf::from(require(&flags, "log")?);
    let socket_path = PathBuf::from(require(&flags, "against")?);

    let report = check_against_socket(&log_path, &socket_path)
        .map_err(|e| CliError::Op(format!("drift check: {}", e)))?;

    let log_hex = hex_encode(&report.log_hash);
    let server_hex = hex_encode(&report.server_hash);
    if report.in_sync() {
        println!(
            "ok: in sync (hash {}, {} records)",
            short_hash(&log_hex),
            report.log_records
        );
        return Ok(());
    }

    println!("DRIFT detected");
    println!(
        "  log replay:    hash {} ({} records)",
        log_hex, report.log_records
    );
    println!(
        "  server state:  hash {} ({} records)",
        server_hex, report.server_records
    );
    println!();
    println!("diffs:");
    for d in &report.diffs {
        match d {
            DriftDiff::OnlyOnServer { entity, id, .. } => {
                println!("  + {} {} (only on server)", entity, id);
            }
            DriftDiff::OnlyInLog { entity, id, .. } => {
                println!("  - {} {} (only in log replay)", entity, id);
            }
            DriftDiff::Tampered {
                entity,
                id,
                log_value,
                server_value,
            } => {
                println!(
                    "  ~ {} {} (tampered)\n      log:    {}\n      server: {}",
                    entity, id, log_value, server_value
                );
            }
        }
    }
    Err(CliError::DriftDetected)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn short_hash(hex: &str) -> String {
    if hex.len() <= 12 {
        hex.to_string()
    } else {
        format!("{}…{}", &hex[..6], &hex[hex.len() - 4..])
    }
}

fn cmd_run(args: &[String]) -> Result<(), CliError> {
    let flags = parse_flags(args, &["log", "module", "socket", "snapshot", "store-path"])?;
    let log_path = PathBuf::from(require(&flags, "log")?);
    let module_dir = PathBuf::from(require(&flags, "module")?);
    let socket_path = PathBuf::from(require(&flags, "socket")?);
    let snapshot_path = flags.get("snapshot").map(PathBuf::from);
    let store_path = flags.get("store-path").map(PathBuf::from);

    eprintln!(
        "nakui run: module={} log={} socket={} snapshot={} store={}",
        module_dir.display(),
        log_path.display(),
        socket_path.display(),
        snapshot_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<none>".into()),
        store_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<memory>".into()),
    );

    // Sidecar brahman: nakui se presenta al Init mientras el daemon vive.
    // No bloquea; si el Init no está, el sidecar termina silenciosamente.
    brahman_sidecar::spawn(brahman_card_for_nakui());

    let executor = Executor::load_module(&module_dir)
        .map_err(|e| CliError::Op(format!("load module {}: {}", module_dir.display(), e)))?;
    let log = EventLog::open(&log_path).map_err(|e| CliError::Op(format!("open log: {}", e)))?;
    let snapshot = match &snapshot_path {
        Some(p) => Some(
            Snapshot::load(p)
                .map_err(|e| CliError::Op(format!("load snapshot: {}", e)))?
                .ok_or_else(|| {
                    CliError::Op(format!("snapshot file does not exist: {}", p.display()))
                })?,
        ),
        None => None,
    };

    if let Some(p) = store_path {
        run_persistent(executor, log, snapshot, &socket_path, &p)
    } else {
        let store = MemoryStore::new();
        run_server(executor, log, store, snapshot, &socket_path)
            .map_err(|e| CliError::Op(format!("run: {}", e)))
    }
}

#[cfg(feature = "persistent")]
fn run_persistent(
    executor: Executor,
    log: EventLog,
    snapshot: Option<Snapshot>,
    socket_path: &std::path::Path,
    store_path: &std::path::Path,
) -> Result<(), CliError> {
    use nakui_core::surreal_store::SurrealStore;
    let store = SurrealStore::new_persistent(store_path).map_err(|e| {
        CliError::Op(format!(
            "open persistent store at {}: {}",
            store_path.display(),
            e
        ))
    })?;
    run_server(executor, log, store, snapshot, socket_path)
        .map_err(|e| CliError::Op(format!("run: {}", e)))
}

#[cfg(not(feature = "persistent"))]
fn run_persistent(
    _executor: Executor,
    _log: EventLog,
    _snapshot: Option<Snapshot>,
    _socket_path: &std::path::Path,
    _store_path: &std::path::Path,
) -> Result<(), CliError> {
    Err(CliError::Op(
        "--store-path requires building with `--features persistent`".into(),
    ))
}

fn cmd_snapshot(args: &[String]) -> Result<(), CliError> {
    let flags = parse_flags(args, &["log", "module", "out"])?;
    let log_path = PathBuf::from(require(&flags, "log")?);
    let module_dir = PathBuf::from(require(&flags, "module")?);
    let out_path = PathBuf::from(require(&flags, "out")?);

    let exec = Executor::load_module(&module_dir)
        .map_err(|e| CliError::Op(format!("load module {}: {}", module_dir.display(), e)))?;
    let log = EventLog::open(&log_path).map_err(|e| CliError::Op(format!("open log: {}", e)))?;
    let mut store = MemoryStore::new();
    replay_with_snapshot_into(&log, None, &mut store)
        .map_err(|e| CliError::Op(format!("replay: {}", e)))?;
    let last_seq = log
        .entries()
        .map_err(|e| CliError::Op(format!("read log: {}", e)))?
        .last()
        .map(|e| e.seq())
        .ok_or_else(|| CliError::Op("log is empty; nothing to snapshot".into()))?;
    let snap = Snapshot::capture(&store, last_seq, &exec);
    snap.write(&out_path)
        .map_err(|e| CliError::Op(format!("write snapshot: {}", e)))?;

    let entity_count: usize = store.records().values().map(|m| m.len()).sum();
    println!(
        "snapshot written to {} (seq {}, {} records, schema {})",
        out_path.display(),
        last_seq,
        entity_count,
        short_hash(&hex_encode(&exec.module_schema_hash())),
    );
    Ok(())
}

fn cmd_compact(args: &[String]) -> Result<(), CliError> {
    let flags = parse_flags(args, &["log", "snapshot"])?;
    let log_path = PathBuf::from(require(&flags, "log")?);
    let snap_path = PathBuf::from(require(&flags, "snapshot")?);

    let snap = Snapshot::load(&snap_path)
        .map_err(|e| CliError::Op(format!("load snapshot: {}", e)))?
        .ok_or_else(|| CliError::Op(format!("snapshot not found: {}", snap_path.display())))?;
    let mut log = EventLog::open(&log_path).map_err(|e| CliError::Op(format!("open log: {}", e)))?;
    let before = log
        .entries()
        .map(|es| es.len())
        .map_err(|e| CliError::Op(format!("read log: {}", e)))?;
    log.compact_through(snap.seq)
        .map_err(|e| CliError::Op(format!("compact: {}", e)))?;
    let after = log
        .entries()
        .map(|es| es.len())
        .map_err(|e| CliError::Op(format!("read log: {}", e)))?;
    println!(
        "compacted {} through seq {} ({} → {} entries; {} dropped)",
        log_path.display(),
        snap.seq,
        before,
        after,
        before.saturating_sub(after),
    );
    Ok(())
}

fn cmd_verify_log(args: &[String]) -> Result<(), CliError> {
    let flags = parse_flags(args, &["log", "module"])?;
    let log_path = PathBuf::from(require(&flags, "log")?);
    let module_dir = PathBuf::from(require(&flags, "module")?);

    let exec = Executor::load_module(&module_dir)
        .map_err(|e| CliError::Op(format!("load module {}: {}", module_dir.display(), e)))?;
    let log = EventLog::open(&log_path).map_err(|e| CliError::Op(format!("open log: {}", e)))?;

    match verify_log(&log, &exec) {
        Ok(()) => {
            let n = log
                .entries()
                .map(|es| es.len())
                .map_err(|e| CliError::Op(format!("read log: {}", e)))?;
            println!("ok: {} entries; every morphism reproduced its ops", n);
            Ok(())
        }
        Err(e) => Err(CliError::Op(format!("verify failed: {}", e))),
    }
}

/// Card que nakui presenta al Init brahman cuando arranca como daemon.
///
/// Lifecycle Daemon (proceso largo). Flujos JSON: consume `command`
/// (queries del UI), produce `report` (resultados de cómputo). Los
/// nombres están escogidos para que el broker pueda matchearlos contra
/// `user-intent` / `render-data` de yahweh-shell por compatibilidad de
/// tipo (todos `json`).
fn brahman_card_for_nakui() -> brahman_card::Card {
    use brahman_card::{
        Card, Flow, Flows, FsPolicy, IpcPolicy, Lifecycle, Payload, Permissions, Priority,
        Supervision, TypeRef, CARD_SCHEMA_VERSION,
    };
    use std::collections::BTreeSet;
    use std::time::Duration;

    Card {
        schema_version: CARD_SCHEMA_VERSION,
        id: ulid::Ulid::new(),
        lineage: None,
        label: "brahman.nakui_erp".into(),
        provides: BTreeSet::new(),
        requires: BTreeSet::new(),
        payload: Payload::Virtual,
        supervision: Supervision::Restart {
            initial: Duration::from_millis(200),
            max: Duration::from_secs(30),
        },
        lifecycle: Lifecycle::Daemon,
        priority: Priority::Normal,
        permissions: Permissions {
            filesystem: FsPolicy::ReadWrite,
            ipc: IpcPolicy {
                allow: vec!["wit-v1".into()],
            },
            ..Default::default()
        },
        flow: Flows {
            input: vec![Flow {
                name: "command".into(),
                ty: TypeRef::Primitive {
                    name: "json".into(),
                },
                pin_to: None,
            }],
            output: vec![Flow {
                name: "report".into(),
                ty: TypeRef::Primitive {
                    name: "json".into(),
                },
                pin_to: None,
            }],
        },
        ..Default::default()
    }
}
