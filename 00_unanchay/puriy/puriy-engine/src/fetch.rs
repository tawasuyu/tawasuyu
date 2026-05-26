//! Net síncrono — `ureq` blocking client.
//!
//! Tomar `ureq` y no `reqwest` es intencional: el engine vive en el hilo
//! del compositor de Llimphi, no necesita scheduler async, y queremos
//! evitar pull de `tokio` en `puriy-engine`. `ureq` es ~7 deps,
//! `reqwest+tokio` es ~80.

use thiserror::Error;
use url::Url;

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
/// Asume `text/html` o `text/*`. Para recursos binarios habría que
/// devolver `Vec<u8>`, pero Fase 2 sólo consume HTML/CSS.
pub fn fetch(url: &Url) -> Result<String, FetchError> {
    let resp = ureq::get(url.as_str())
        .set("User-Agent", concat!("puriy/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => FetchError::Status(code),
            ureq::Error::Transport(t) => FetchError::Transport(t.to_string()),
        })?;
    resp.into_string().map_err(|e| FetchError::Body(e.to_string()))
}
