//! `minga-cli`: subcomandos del CLI de Minga.
//!
//! La CLI expone funciones puras (`commands`) que retornan `Result`
//! con la información estructurada. El binario `minga` (en `main.rs`)
//! solo parsea argumentos, prompts de passphrase, y formatea la
//! salida. Esa separación hace los comandos directamente testeables
//! sin spawn de subprocesos.

pub mod commands;
pub mod error;

pub use commands::{
    cmd_ingest, cmd_init, cmd_listen, cmd_status, cmd_sync, cmd_watch, IngestResult, RepoStatus,
};
pub use error::CliError;
