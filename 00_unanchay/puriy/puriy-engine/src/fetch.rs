//! Net síncrono — `ureq` blocking client.
//!
//! Tomar `ureq` y no `reqwest` es intencional: el engine vive en el hilo
//! del compositor de Llimphi, no necesita scheduler async, y queremos
//! evitar pull de `tokio` en `puriy-engine`. `ureq` es ~7 deps,
//! `reqwest+tokio` es ~80.

use thiserror::Error;
use url::Url;

use crate::cache;

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("transporte: {0}")]
    Transport(String),
    #[error("status HTTP {0}")]
    Status(u16),
    #[error("body no-UTF8: {0}")]
    Body(String),
}

/// GET sobre la URL dada; devuelve el body como String.
///
/// Pasa por la cache global: si la URL ya fue descargada antes en este
/// proceso, sale instantáneo sin tocar la red. Si miss, descarga,
/// guarda en cache y devuelve.
pub fn fetch(url: &Url) -> Result<String, FetchError> {
    let bytes = fetch_bytes(url.as_str())?;
    String::from_utf8(bytes).map_err(|e| FetchError::Body(e.to_string()))
}

/// Versión que devuelve bytes — útil para assets binarios (imágenes).
/// Mismo mecanismo de cache.
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    if let Some(hit) = cache::get(url) {
        return Ok(hit);
    }
    let resp = ureq::get(url)
        .set("User-Agent", concat!("puriy/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => FetchError::Status(code),
            ureq::Error::Transport(t) => FetchError::Transport(t.to_string()),
        })?;
    let mut bytes = Vec::new();
    use std::io::Read;
    resp.into_reader()
        .take(64 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| FetchError::Transport(e.to_string()))?;
    cache::put(url, bytes.clone());
    Ok(bytes)
}
