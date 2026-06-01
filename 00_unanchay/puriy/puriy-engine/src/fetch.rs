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
    fetch_with_referer(url, None)
}

/// Como `fetch` pero acepta un `referer` opcional — la URL desde la
/// que se navega. Sigue el patrón de los browsers: enviar Referer
/// SIEMPRE para http/https; el strip-on-cross-origin queda fuera de
/// scope por ahora (matchea el "no referrer policy" default antiguo).
pub fn fetch_with_referer(
    url: &Url,
    referer: Option<&str>,
) -> Result<(String, String), FetchError> {
    let (bytes, final_url) = fetch_bytes_with_referer(url.as_str(), referer)?;
    let html = String::from_utf8(bytes).map_err(|e| FetchError::Body(e.to_string()))?;
    Ok((html, final_url))
}

/// Versión que devuelve bytes — útil para assets binarios (imágenes).
/// Mismo mecanismo de cache. Lee `Cache-Control: max-age=N` para
/// computar el `expires_at`; si no hay header, la entrada se guarda
/// sin TTL (sólo expira por eviction LRU).
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    Ok(fetch_bytes_with_referer(url, None)?.0)
}

/// Como `fetch_bytes` pero también devuelve la URL final tras seguir
/// redirects. La cache se indexa por la URL **solicitada** (no la
/// final) para que un mismo permalink siempre sirva del mismo slot.
pub fn fetch_bytes_with_url(url: &str) -> Result<(Vec<u8>, String), FetchError> {
    fetch_bytes_with_referer(url, None)
}

/// Variante completa que acepta un referer opcional. El header
/// Referer se setea sólo si la URL fuente parsea como http/https
/// (queremos evitar fugar `file://` o `about:` schemes).
pub fn fetch_bytes_with_referer(
    url: &str,
    referer: Option<&str>,
) -> Result<(Vec<u8>, String), FetchError> {
    // file:// — páginas (y assets) locales del disco. Un navegador debe
    // poder abrir un `.html` local y resolver sus `src`/`href` relativos
    // (que el engine ya transforma en `file://…`). No pasa por la cache:
    // un archivo en disco se relee siempre fresco. El resto del pipeline
    // (parse/style/layout) es agnóstico al origen de los bytes.
    if url.starts_with("file://") {
        let path = url::Url::parse(url)
            .ok()
            .and_then(|u| u.to_file_path().ok())
            .unwrap_or_else(|| std::path::PathBuf::from(url.trim_start_matches("file://")));
        let bytes = std::fs::read(&path)
            .map_err(|e| FetchError::Transport(format!("file {}: {e}", path.display())))?;
        return Ok((bytes, url.to_string()));
    }
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
    if let Some(r) = sanitize_referer(referer) {
        req = req.set("Referer", &r);
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

/// Fase 7.31 — request HTTP arbitrario para el `fetch()` JS. Devuelve
/// status code + body + headers + final_url. NO usa cache (el JS fetch
/// puede tener semánticas Cache-Control distintas; conservador
/// no-cachear). Headers se filtran para no incluir Set-Cookie (las
/// cookies se aplican aparte vía `crate::cookies`).
#[derive(Debug, Clone)]
pub struct FetchResponse {
    pub status: u16,
    pub status_text: String,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
    pub final_url: String,
}

/// Versión "full" del request HTTP para alimentar `fetch()` desde JS.
/// Acepta method arbitrario (GET/POST/PUT/DELETE/PATCH/HEAD/OPTIONS),
/// body opcional y headers extra. Devuelve `FetchResponse` con status,
/// headers, body. Errores de transporte se mapean a `FetchError::Transport`;
/// status no-2xx **NO** se traduce a `Status` — se devuelve igual con el
/// código y body, igual que `window.fetch` real (sólo rechaza Promise
/// en errores de network, no por status HTTP).
pub fn fetch_full(
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    headers: &[(String, String)],
) -> Result<FetchResponse, FetchError> {
    let parsed = url::Url::parse(url).ok();
    let host = parsed.as_ref().and_then(|u| u.host_str()).map(|s| s.to_string());
    let method_upper = method.to_ascii_uppercase();
    let mut req = match method_upper.as_str() {
        "GET" => agent().get(url),
        "POST" => agent().post(url),
        "PUT" => agent().put(url),
        "DELETE" => agent().delete(url),
        "PATCH" => agent().request("PATCH", url),
        "HEAD" => agent().head(url),
        "OPTIONS" => agent().request("OPTIONS", url),
        other => agent().request(other, url),
    };
    req = req.set("User-Agent", concat!("puriy/", env!("CARGO_PKG_VERSION")));
    if let Some(h) = host.as_deref() {
        if let Some(cookie_hdr) = crate::cookies::cookie_header(h) {
            req = req.set("Cookie", &cookie_hdr);
        }
    }
    for (k, v) in headers {
        req = req.set(k, v);
    }
    // ureq mapea send/call según si hay body o no.
    let result = match body {
        Some(b) => req.send_bytes(b),
        None => req.call(),
    };
    // En `fetch()` (spec) un status no-2xx NO rechaza la Promise — se
    // devuelve igual con `response.ok = false`. ureq lo modela como
    // `Error::Status` para conveniencia; lo desunwrappeamos.
    let resp = match result {
        Ok(r) => r,
        Err(ureq::Error::Status(_code, r)) => r,
        Err(ureq::Error::Transport(t)) => return Err(FetchError::Transport(t.to_string())),
    };
    let status = resp.status();
    let status_text = resp.status_text().to_string();
    let final_url = resp.get_url().to_string();
    let header_names: Vec<String> = resp.headers_names();
    let mut headers_out: Vec<(String, String)> = Vec::new();
    for name in &header_names {
        // Skip Set-Cookie del response visible — las cookies van por
        // su propio canal (puriy-engine::cookies). Esto matchea spec:
        // `Headers` del fetch normalmente NO expone Set-Cookie.
        if name.eq_ignore_ascii_case("set-cookie") {
            continue;
        }
        if let Some(v) = resp.header(name) {
            headers_out.push((name.to_ascii_lowercase(), v.to_string()));
        }
    }
    // Aplicar cookies del response (mismo molde que fetch_bytes_with_referer).
    if let Some(h) = host.as_deref() {
        for sc in resp.all("Set-Cookie") {
            crate::cookies::put_set_cookie(h, sc);
        }
    }
    let mut body_bytes = Vec::new();
    use std::io::Read;
    resp.into_reader()
        .take(64 * 1024 * 1024)
        .read_to_end(&mut body_bytes)
        .map_err(|e| FetchError::Transport(e.to_string()))?;
    Ok(FetchResponse {
        status,
        status_text,
        body: body_bytes,
        headers: headers_out,
        final_url,
    })
}

/// POST con body `application/x-www-form-urlencoded`. NO usa cache —
/// los POST son no-idempotentes. Devuelve `(body, final_url)` del
/// response, siguiendo redirects 3xx (ureq convierte 301/302/303 a GET
/// hacia el `Location:` igual que un browser real).
pub fn post_form(url: &str, body: &str) -> Result<(String, String), FetchError> {
    post_form_with_referer(url, body, None)
}

/// POST con `Referer` opcional — la URL desde la que se navega.
pub fn post_form_with_referer(
    url: &str,
    body: &str,
    referer: Option<&str>,
) -> Result<(String, String), FetchError> {
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
    if let Some(r) = sanitize_referer(referer) {
        req = req.set("Referer", &r);
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

/// Decide qué valor mandar como `Referer:`. Aceptamos sólo URLs
/// http/https — `about:`, `file:`, `data:` y similares no deben fugarse
/// al server. Cualquier fragment se strippea (es información del
/// cliente, nunca debe viajar al server). El query se preserva.
fn sanitize_referer(referer: Option<&str>) -> Option<String> {
    let r = referer?;
    let parsed = url::Url::parse(r).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let mut clean = parsed.clone();
    clean.set_fragment(None);
    Some(clean.to_string())
}

fn now_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_url_lee_archivo_local() {
        // Un .html en disco se abre por file:// y devuelve sus bytes tal cual.
        let mut path = std::env::temp_dir();
        path.push(format!("puriy_file_test_{}.html", std::process::id()));
        let html = b"<!doctype html><h1>hola local</h1>";
        std::fs::write(&path, html).expect("escribir temp");
        let url = format!("file://{}", path.display());
        let (bytes, final_url) = fetch_bytes_with_referer(&url, None).expect("leer file://");
        assert_eq!(bytes, html);
        assert_eq!(final_url, url);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_url_inexistente_da_error_transport() {
        let url = "file:///no/existe/jamas_de_los_jamases_puriy.html";
        assert!(matches!(
            fetch_bytes_with_referer(url, None),
            Err(FetchError::Transport(_))
        ));
    }

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
    fn sanitize_referer_acepta_http_y_https() {
        assert_eq!(
            sanitize_referer(Some("https://example.com/p?q=1")).as_deref(),
            Some("https://example.com/p?q=1")
        );
        assert_eq!(
            sanitize_referer(Some("http://example.com/")).as_deref(),
            Some("http://example.com/")
        );
    }

    #[test]
    fn sanitize_referer_strippea_fragment() {
        assert_eq!(
            sanitize_referer(Some("https://example.com/p#section")).as_deref(),
            Some("https://example.com/p")
        );
    }

    #[test]
    fn sanitize_referer_rechaza_no_http() {
        assert_eq!(sanitize_referer(Some("about:blank")), None);
        assert_eq!(sanitize_referer(Some("file:///etc/passwd")), None);
        assert_eq!(sanitize_referer(Some("data:text/html,x")), None);
        assert_eq!(sanitize_referer(None), None);
        // URL inválida → None.
        assert_eq!(sanitize_referer(Some("not a url")), None);
    }

    #[test]
    fn parse_max_age_sin_directiva_devuelve_none() {
        assert_eq!(parse_max_age("public"), None);
        assert_eq!(parse_max_age(""), None);
    }
}
