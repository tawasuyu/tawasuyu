//! Descarga de `<script src="...">` externos en paralelo.
//!
//! Fase 7.4: el DOM ya recolecta `ScriptInfo` con `src: Some(...)` para
//! scripts externos (Fase 7.0). Acá los resolvemos contra la URL base
//! del documento, los filtramos a http/https, y descargamos los cuerpos
//! en un pool de workers (mismo patrón que `prefetch_image_urls`).
//!
//! El body UTF-8 se inyecta en `ScriptInfo.inline` — para el chrome,
//! un externo ya descargado es indistinguible de un inline. Esto
//! preserva el contrato del runtime de Fase 7.x (clásico, no module
//! loader) y deja `src` intacto por si el caller quiere mostrarlo.

use std::sync::{Arc, Mutex};

use url::Url;

use crate::dom::ScriptInfo;
use crate::fetch;

const SCRIPT_FETCH_WORKERS: usize = 6;

/// Resuelve cada `<script src="...">` no-module contra `base` y descarga
/// el cuerpo en paralelo. Llena `ScriptInfo.inline` con el body UTF-8.
///
/// - Scripts con `is_module=true` se saltean (el runtime de Fase 7.x es
///   clásico).
/// - Scripts cuyo `src` resuelve a un scheme no-http (`data:`/`file:`/
///   `about:`) se saltean — `data:application/javascript,...` quedará
///   para una fase posterior.
/// - Scripts cuyo fetch falla quedan con `inline = None` y los saltea el
///   chrome (no se reportan como error JS — son network failures, no
///   syntax/runtime errors).
/// - Scripts que ya traen `inline` (raros, pero válidos en spec si el
///   parser se equivocó) no se tocan.
pub fn fetch_externals(scripts: &mut [ScriptInfo], base: &str) {
    let Ok(base) = Url::parse(base) else {
        return;
    };
    let targets = collect_external_targets(scripts, &base);
    if targets.is_empty() {
        return;
    }

    // Dispara workers en chunks fijos (mismo patrón que
    // `prefetch_image_urls`). Sin rayon — std::thread alcanza para los
    // 1-30 scripts típicos por página.
    let chunk_size = targets.len().div_ceil(SCRIPT_FETCH_WORKERS).max(1);
    let chunks: Vec<Vec<(usize, String)>> = targets
        .chunks(chunk_size)
        .map(|c| c.to_vec())
        .collect();
    let results: Arc<Mutex<Vec<(usize, Result<String, ()>)>>> =
        Arc::new(Mutex::new(Vec::with_capacity(targets.len())));
    let mut handles = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let results = results.clone();
        handles.push(std::thread::spawn(move || {
            for (idx, url) in chunk {
                let res = fetch::fetch_bytes(&url)
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .ok_or(());
                results.lock().expect("poisoned").push((idx, res));
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }

    let mut guard = results.lock().expect("poisoned");
    for (idx, res) in guard.drain(..) {
        if let Ok(body) = res {
            if let Some(s) = scripts.get_mut(idx) {
                s.inline = Some(body);
            }
        }
    }
}

/// Walka `scripts` y devuelve `(índice, URL absoluta)` para cada
/// candidato a fetch externo. Útil aislado de la red para testear la
/// lógica de filtrado y resolución sin tocar sockets.
pub(crate) fn collect_external_targets(
    scripts: &[ScriptInfo],
    base: &Url,
) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (i, s) in scripts.iter().enumerate() {
        if s.is_module {
            continue;
        }
        if s.inline.is_some() {
            continue;
        }
        let Some(src) = &s.src else {
            continue;
        };
        let trimmed = src.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(abs) = base.join(trimmed) else {
            continue;
        };
        if !matches!(abs.scheme(), "http" | "https") {
            continue;
        }
        out.push((i, abs.into()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(src: Option<&str>, inline: Option<&str>, is_module: bool) -> ScriptInfo {
        ScriptInfo {
            src: src.map(|s| s.to_string()),
            inline: inline.map(|s| s.to_string()),
            type_attr: None,
            is_module,
            defer: false,
            async_: false,
        }
    }

    #[test]
    fn collect_resuelve_relativos_contra_base() {
        let base = Url::parse("https://example.com/sub/").unwrap();
        let scripts = vec![
            script(Some("/abs.js"), None, false),
            script(Some("rel.js"), None, false),
            script(Some("../up.js"), None, false),
        ];
        let targets = collect_external_targets(&scripts, &base);
        assert_eq!(
            targets,
            vec![
                (0, "https://example.com/abs.js".to_string()),
                (1, "https://example.com/sub/rel.js".to_string()),
                (2, "https://example.com/up.js".to_string()),
            ]
        );
    }

    #[test]
    fn collect_filtra_no_http() {
        let base = Url::parse("https://example.com/").unwrap();
        let scripts = vec![
            script(Some("data:application/javascript,var%20x=1"), None, false),
            script(Some("file:///tmp/x.js"), None, false),
            script(Some("about:blank"), None, false),
            script(Some("javascript:void(0)"), None, false),
            script(Some("https://cdn.example/ok.js"), None, false),
        ];
        let targets = collect_external_targets(&scripts, &base);
        // Sólo el último (https://) sobrevive.
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, 4);
    }

    #[test]
    fn collect_saltea_modules() {
        let base = Url::parse("https://example.com/").unwrap();
        let scripts = vec![
            script(Some("/m.js"), None, true),
            script(Some("/clasico.js"), None, false),
        ];
        let targets = collect_external_targets(&scripts, &base);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, 1);
    }

    #[test]
    fn collect_saltea_sin_src() {
        let base = Url::parse("https://example.com/").unwrap();
        let scripts = vec![
            script(None, Some("console.log(1)"), false),
            script(Some(""), None, false),
            script(Some("   "), None, false),
        ];
        let targets = collect_external_targets(&scripts, &base);
        assert!(targets.is_empty());
    }

    #[test]
    fn collect_saltea_si_ya_hay_inline() {
        // Si `inline` ya está poblado, no re-fetcheamos (idempotente).
        let base = Url::parse("https://example.com/").unwrap();
        let scripts = vec![script(Some("/x.js"), Some("already"), false)];
        let targets = collect_external_targets(&scripts, &base);
        assert!(targets.is_empty());
    }

    #[test]
    fn fetch_externals_base_invalida_no_panic() {
        // Sin URL parseable, no hace nada — no se trapea en
        // `Engine::load_with_referer` después de un load exitoso, pero
        // sirve como defensa.
        let mut scripts = vec![script(Some("/x.js"), None, false)];
        fetch_externals(&mut scripts, "not-a-url");
        assert_eq!(scripts[0].inline, None);
    }

    #[test]
    fn fetch_externals_sin_candidatos_no_lanza_threads() {
        // Sólo inline + modules + data: → no hay nada que fetchear, la
        // función debe retornar inmediato sin tocar la red.
        let mut scripts = vec![
            script(None, Some("var x = 1"), false),
            script(Some("/m.js"), None, true),
            script(Some("data:text/javascript,1"), None, false),
        ];
        let t0 = std::time::Instant::now();
        fetch_externals(&mut scripts, "https://example.com/");
        let dt = t0.elapsed();
        assert!(
            dt < std::time::Duration::from_millis(50),
            "esperaba retorno inmediato sin fetch, tardó {:?}",
            dt
        );
        // Ninguno de los originales recibió inline nuevo.
        assert_eq!(scripts[0].inline.as_deref(), Some("var x = 1"));
        assert!(scripts[1].inline.is_none());
        assert!(scripts[2].inline.is_none());
    }
}
