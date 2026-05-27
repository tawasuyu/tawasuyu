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
/// Mismo mecanismo de cache. Lee `Cache-Control: max-age=N` para
/// computar el `expires_at`; si no hay header, la entrada se guarda
/// sin TTL (sólo expira por eviction LRU).
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
    let cc = resp.header("Cache-Control").map(|s| s.to_string());
    let mut bytes = Vec::new();
    use std::io::Read;
    resp.into_reader()
        .take(64 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| FetchError::Transport(e.to_string()))?;
    let expires_at = cc
        .as_deref()
        .and_then(parse_max_age)
        .map(|max_age| now_unix().saturating_add(max_age))
        .unwrap_or(u64::MAX);
    cache::put_with_expiry(url, bytes.clone(), expires_at);
    Ok(bytes)
}

/// Parser minimal de `Cache-Control: max-age=N`. Ignora `s-maxage`,
/// `no-store`, etc. — esos directivos requerirían lógica adicional que
/// queda fuera del scope inicial. Devuelve `None` si:
/// - el header tiene `no-store` o `no-cache` (rechaza cachear con TTL),
/// - no aparece `max-age=` con un entero positivo.
fn parse_max_age(cc: &str) -> Option<u64> {
    let lower = cc.to_ascii_lowercase();
    if lower.contains("no-store") || lower.contains("no-cache") {
        return None;
    }
    for tok in lower.split(',') {
        let tok = tok.trim();
        if let Some(rest) = tok.strip_prefix("max-age=") {
            return rest.parse::<u64>().ok();
        }
    }
    None
}

fn now_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_max_age_extrae_segundos() {
        assert_eq!(parse_max_age("max-age=3600"), Some(3600));
        assert_eq!(parse_max_age("public, max-age=604800"), Some(604_800));
        assert_eq!(parse_max_age("max-age=0"), Some(0));
    }

    #[test]
    fn parse_max_age_rechaza_no_store_no_cache() {
        assert_eq!(parse_max_age("no-store, max-age=60"), None);
        assert_eq!(parse_max_age("no-cache"), None);
    }

    #[test]
    fn parse_max_age_sin_directiva_devuelve_none() {
        assert_eq!(parse_max_age("public"), None);
        assert_eq!(parse_max_age(""), None);
    }
}
