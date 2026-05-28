//! `minga-cli`: subcomandos del CLI de Minga.
//!
//! La CLI expone funciones puras (`commands`) que retornan `Result`
//! con la información estructurada. El binario `minga` (en `main.rs`)
//! solo parsea argumentos, prompts de passphrase, y formatea la
//! salida. Esa separación hace los comandos directamente testeables
//! sin spawn de subprocesos.

pub mod bundle;
pub mod commands;
pub mod error;
pub mod serve;

pub use bundle::{
    cmd_bundle_export, cmd_bundle_export_all, cmd_bundle_import, cmd_bundle_import_all,
    BundleExportAllStats, BundleExportStats, BundleImportAllStats, BundleImportStats,
};
pub use commands::{
    cmd_blame, cmd_diff, cmd_history, cmd_ingest, cmd_ingest_dir, cmd_init, cmd_listen, cmd_log,
    cmd_mount, cmd_prune, cmd_retire, cmd_roots, cmd_show, cmd_sign, cmd_signers, cmd_status,
    cmd_sync, cmd_verify_root, cmd_watch, BlameLine, BulkIngestStats, DiffLine, DiffResult,
    HistoryEntry, IngestResult, LogEntry, PruneStats, RepoStatus, RetireResult, RootRow,
    ShowResult, SignResult, SignerEntry, VerifyResult,
};
pub use error::CliError;
pub use serve::cmd_serve;
