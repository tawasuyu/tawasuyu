//! Cliente admin: lee un `StatusSnapshot` desde un socket admin.

use std::path::Path;

use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixStream;

use crate::snapshot::StatusSnapshot;

#[derive(Debug, Error)]
pub enum AdminError {
    #[error("E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("respuesta vacía")]
    Empty,
    #[error("JSON inválido: {0}")]
    Json(#[from] serde_json::Error),
}

/// Conecta al socket admin, lee la línea JSON y deserializa.
pub async fn query(path: impl AsRef<Path>) -> Result<StatusSnapshot, AdminError> {
    let stream = UnixStream::connect(path).await?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Err(AdminError::Empty);
    }
    let snapshot = serde_json::from_str(&line)?;
    Ok(snapshot)
}

/// Variante sync de [`query`] para callers que no tienen runtime tokio
/// (típicamente: GUIs con su propio executor, como GPUI).
pub fn query_blocking(path: impl AsRef<Path>) -> Result<StatusSnapshot, AdminError> {
    use std::io::{BufRead, BufReader as StdBufReader};
    use std::os::unix::net::UnixStream as StdUnixStream;
    let stream = StdUnixStream::connect(path)?;
    let mut reader = StdBufReader::new(stream);
    let mut line = String::new();
    let n = reader.read_line(&mut line)?;
    if n == 0 {
        return Err(AdminError::Empty);
    }
    let snapshot = serde_json::from_str(&line)?;
    Ok(snapshot)
}
