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

/// Agente ureq con redirects extendidos (default ureq es 5; lo subimos a
/// 10 para tolerar cadenas largas tipo url-shortener → http→https →
/// trailing-slash → www→apex). `redirects(0)` lo desactivaría.
fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().redirects(10).build()
}

/// GET sobre la URL dada; devuelve `(html, final_url)`. La URL final
/// puede diferir de la solicitada si el server redirigió (3xx) — el
/// engine la usa como base para resolver hrefs relativos y la chrome la
/// muestra en la barra. Pasa por la cache global.
pub fn fetch(url: &Url) -> Result<(String, String), FetchError> {
    let (bytes, final_url) = fetch_bytes_with_url(url.as_str())?;
    let html = String::from_utf8(bytes).map_err(|e| FetchError::Body(e.to_string()))?;
    Ok((html, final_url))
}

/// Versión que devuelve bytes — útil para assets binarios (imágenes).
/// Mismo mecanismo de cache. Lee `Cache-Control: max-age=N` para
/// computar el `expires_at`; si no hay header, la entrada se guarda
/// sin TTL (sólo expira por eviction LRU).
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    Ok(fetch_bytes_with_url(url)?.0)
}

/// Como `fetch_bytes` pero también devuelve la URL final tras seguir
/// redirects. La cache se indexa por la URL **solicitada** (no la
/// final) para que un mismo permalink siempre sirva del mismo slot.
pub fn fetch_bytes_with_url(url: &str) -> Result<(Vec<u8>, String), FetchError> {
    if let Some(hit) = cache::get(url) {
        return Ok((hit, url.to_string()));
    }
    let parsed = url::Url::parse(url).ok();
    let host = parsed.as_ref().and_then(|u| u.host_str()).map(|s| s.to_string());
    let mut req = agent()
        .get(url)
        .set("User-Agent", concat!("puriy/", env!("CARGO_PKG_VERSION")));
    if let Some(h) = host.as_deref() {
        if let Some(cookie_hdr) = crate::cookies::cookie_header(h) {
            req = req.set("Cookie", &cookie_hdr);
        }
    }
    let resp = req.call().map_err(|e| match e {
        ureq::Error::Status(code, _) => FetchError::Status(code),
        ureq::Error::Transport(t) => FetchError::Transport(t.to_string()),
    })?;
    let final_url = resp.get_url().to_string();
    let cc = resp.header("Cache-Control").map(|s| s.to_string());
    // Set-Cookie: ureq junta headers en `resp.all("Set-Cookie")`.
    if let Some(h) = host.as_deref() {
        for sc in resp.all("Set-Cookie") {
            crate::cookies::put_set_cookie(h, sc);
        }
    }
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
    Ok((bytes, final_url))
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

/// POST con body `application/x-www-form-urlencoded`. NO usa cache —
/// los POST son no-idempotentes. Devuelve `(body, final_url)` del
/// response, siguiendo redirects 3xx (ureq convierte 301/302/303 a GET
/// hacia el `Location:` igual que un browser real).
pub fn post_form(url: &str, body: &str) -> Result<(String, String), FetchError> {
    let parsed = url::Url::parse(url).ok();
    let host = parsed.as_ref().and_then(|u| u.host_str()).map(|s| s.to_string());
    let mut req = agent()
        .post(url)
        .set("User-Agent", concat!("puriy/", env!("CARGO_PKG_VERSION")))
        .set("Content-Type", "application/x-www-form-urlencoded");
    if let Some(h) = host.as_deref() {
        if let Some(cookie_hdr) = crate::cookies::cookie_header(h) {
            req = req.set("Cookie", &cookie_hdr);
        }
    }
    let resp = req.send_string(body).map_err(|e| match e {
        ureq::Error::Status(code, _) => FetchError::Status(code),
        ureq::Error::Transport(t) => FetchError::Transport(t.to_string()),
    })?;
    let final_url = resp.get_url().to_string();
    if let Some(h) = host.as_deref() {
        for sc in resp.all("Set-Cookie") {
            crate::cookies::put_set_cookie(h, sc);
        }
    }
    let body_str = resp.into_string().map_err(|e| FetchError::Transport(e.to_string()))?;
    Ok((body_str, final_url))
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
