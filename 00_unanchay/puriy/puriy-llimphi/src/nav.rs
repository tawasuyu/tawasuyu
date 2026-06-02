//! Navegación y red: arranque de cargas (`start_load`/`start_load_post`),
//! workers async de fetch/load/EventSource (`spawn_*`), parseo del payload de
//! `fetch` publicado por el JS, resolución de valores de `<select>` y del texto
//! del body, y la lógica de nombre/ruta de descargas. Extraído de `lib.rs`
//! (regla #1). Comparte los tipos del crate vía `use super::*`.
use super::*;

/// Fase 7.31 — parsea el payload del kind 'fetch' publicado por el JS.
/// Formato: campos separados por U+001D — [id, method, url, has_body_flag,
/// body, h_name1, h_val1, h_name2, h_val2, ...]. Devuelve `Msg::FetchRequest`
/// o `None` si el payload es malformado.
pub(crate) fn parse_fetch_payload(value: &str, tab: TabId, gen: u64) -> Option<Msg> {
    let parts: Vec<&str> = value.split('\u{001D}').collect();
    if parts.len() < 5 {
        return None;
    }
    let fetch_id: u32 = parts[0].parse().ok()?;
    let method = parts[1].to_string();
    let url = parts[2].to_string();
    let has_body = parts[3] == "1";
    let body = if has_body {
        Some(parts[4].as_bytes().to_vec())
    } else {
        None
    };
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut i = 5;
    while i + 1 < parts.len() {
        headers.push((parts[i].to_string(), parts[i + 1].to_string()));
        i += 2;
    }
    Some(Msg::FetchRequest { tab, gen, fetch_id, method, url, body, headers })
}

/// Spawn worker thread que ejecuta el fetch HTTP y devuelve
/// `Msg::FetchComplete` al main loop. Mismo molde que `spawn_load`.
pub(crate) fn spawn_fetch(req: Msg, handle: Handle<Msg>) {
    let Msg::FetchRequest { tab, gen, fetch_id, method, url, body, headers } = req else {
        return;
    };
    std::thread::spawn(move || {
        let result = puriy_engine::fetch::fetch_full(
            &method,
            &url,
            body.as_deref(),
            &headers,
        )
        .map_err(|e| e.to_string());
        handle.dispatch(Msg::FetchComplete { tab, gen, fetch_id, result });
    });
}

/// Devuelve el value (del option seleccionado) del `<select>` del slot
/// `select_idx` cuando el option seleccionado es `opt_idx`. Walka el
/// BoxTree contando selects en DFS, mismo orden que el populado en
/// `Msg::Loaded`. None si el slot/opt no existe.
pub(crate) fn select_value_at(t: &TabState, select_idx: usize, opt_idx: usize) -> Option<String> {
    let bt = t.box_tree.as_ref()?;
    let mut counter = 0usize;
    let mut found: Option<String> = None;
    bt.walk(|b| {
        if let Some(s) = &b.select {
            if counter == select_idx {
                found = s.options.get(opt_idx).map(|o| o.value.clone());
            }
            counter += 1;
        }
    });
    found
}

/// Busca el índice del option dentro del `<select>` del slot `select_idx`
/// cuyo `value` coincide con `target` (case-sensitive, exact match).
/// Walka el BoxTree contando selects en DFS. Devuelve None si no existe
/// el slot o ningún option matchea.
pub(crate) fn select_option_index_by_value(bt: &BoxTree, select_idx: usize, target: &str) -> Option<usize> {
    let mut counter = 0usize;
    let mut found: Option<usize> = None;
    bt.walk(|b| {
        if let Some(s) = &b.select {
            if counter == select_idx {
                found = s.options.iter().position(|o| o.value == target);
            }
            counter += 1;
        }
    });
    found
}

/// Concatena las hojas de texto del box tree en un único string — el
/// `body.textContent` que ve el JS via `document.body.textContent`.
/// Separa con un espacio entre nodos para evitar que palabras de
/// nodos adyacentes se peguen.
pub(crate) fn extract_body_text(bt: &BoxTree) -> String {
    let mut out = String::new();
    bt.walk(|b| {
        if let Some(text) = &b.text {
            if !text.is_empty() {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(text);
            }
        }
    });
    out
}

/// Inicia la carga de `url` en la pestaña activa. Si `push_history` es
/// `true`, se trunca y empuja al stack — útil para Navigate; back/fwd/
/// reload pasan `false`.
pub(crate) fn start_load(m: &mut Model, url: String, push_history: bool, handle: &Handle<Msg>) {
    let now_ms = m.start.elapsed().as_millis() as u64;
    let t = m.active_mut();
    // Fase 7.41 — antes de pisar la URL, disparar `beforeunload` para que
    // apps con formularios sin guardar / analytics puedan hacer cleanup
    // (`window.addEventListener('beforeunload', fn)`). Las mutaciones DOM
    // que el handler haga se aplican sobre la página vieja (se va a tirar
    // a la basura, no importa). **Divergencia documentada**: el spec real
    // exige confirmación al usuario si el handler setea `returnValue` o
    // llama `preventDefault()`; acá no hay diálogo modal — siempre se
    // navega.
    if t.js.is_some() {
        let (_, pending) = dispatch_window_js_event_on_tab(t, "beforeunload", now_ms);
        for req in pending {
            handle.dispatch(req);
        }
    }
    // El referer es la URL desde la que se navega — útil para que el
    // server sepa de dónde viene el click. Capturado ANTES de pisar t.url.
    let referer = if t.url == NEW_TAB_URL || t.url.is_empty() { None } else { Some(t.url.clone()) };
    t.url = url.clone();
    t.addr.set_text(url.clone());
    t.addr_focused = false;
    t.status = format!("cargando {url}…");
    t.scroll_y = 0.0;
    t.box_tree = None;
    if push_history {
        // Trunca lo que esté adelante del cursor — convención estándar.
        t.history.truncate(t.cursor + 1);
        if t.history.last() != Some(&url) {
            t.history.push(url.clone());
            t.cursor = t.history.len() - 1;
        }
    }
    t.gen = t.gen.wrapping_add(1);
    let (id, gen) = (t.id, t.gen);
    spawn_load(id, gen, url, referer, current_viewport(), handle.clone());
}

/// Viewport real actual (px físicos + DPR), leído de los thread-locals en el
/// hilo main. Se captura ANTES de spawnear el worker (que no ve los TLS) para
/// que el engine resuelva los `@media` del documento contra la ventana real.
pub(crate) fn current_viewport() -> puriy_engine::Viewport {
    let (w, h) = PURIY_VIEWPORT.with(|c| c.get());
    puriy_engine::Viewport { width: w, height: h, dpr: PURIY_DPR.with(|c| c.get()) as f32 }
}

pub(crate) fn spawn_load(
    tab: TabId,
    gen: u64,
    url: String,
    referer: Option<String>,
    viewport: puriy_engine::Viewport,
    handle: Handle<Msg>,
) {
    if url == NEW_TAB_URL {
        // No fetch para about:blank.
        return;
    }
    std::thread::spawn(move || {
        let engine = Engine::new().with_viewport(viewport);
        match engine.load_with_referer(&url, referer.as_deref()) {
            Ok(doc) => {
                let title = if doc.title.is_empty() { doc.url.clone() } else { doc.title.clone() };
                handle.dispatch(Msg::Loaded {
                    tab,
                    gen,
                    final_url: doc.url.clone(),
                    title,
                    box_tree: doc.box_tree,
                    source: doc.source,
                    meta_refresh: doc.meta_refresh,
                    scripts: doc.scripts,
                });
                // Best-effort: persistimos la cache después de cada
                // navegación exitosa. Si el proceso muere por SIGKILL o
                // panic, sólo se pierde la navegación en vuelo — las
                // anteriores ya quedaron en disco.
                puriy_engine::cache::flush();
            }
            Err(e) => handle.dispatch(Msg::LoadFailed { tab, gen, err: e.to_string() }),
        }
    });
}

/// Worker de un `EventSource` (Fase 7.182): corre el stream SSE en un thread
/// dedicado y reinyecta cada evento al runtime vía `Msg::EsDispatch`. El
/// `cancel` (compartido con `TabState.es_cancel`) lo corta en `close()` o al
/// navegar. La reconexión/parseo viven en `puriy_engine::sse::run_eventsource`.
pub(crate) fn spawn_eventsource(
    tab: TabId,
    gen: u64,
    es_id: u32,
    url: String,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Handle<Msg>,
) {
    std::thread::spawn(move || {
        let cancelled = || cancel.load(std::sync::atomic::Ordering::Relaxed);
        let emit = |kind: &str, ev: Option<&puriy_engine::sse::SseEvent>| {
            handle.dispatch(Msg::EsDispatch {
                tab,
                gen,
                es_id,
                kind: kind.to_string(),
                event_type: ev.map(|e| e.event_type.clone()).unwrap_or_default(),
                data: ev.map(|e| e.data.clone()).unwrap_or_default(),
                last_id: ev.map(|e| e.last_id.clone()).unwrap_or_default(),
            });
        };
        puriy_engine::sse::run_eventsource(
            &url,
            &cancelled,
            || emit("open", None),
            |ev| emit("message", Some(ev)),
            || emit("error", None),
        );
    });
}

pub(crate) fn start_load_post(m: &mut Model, url: String, body: String, handle: &Handle<Msg>) {
    let now_ms = m.start.elapsed().as_millis() as u64;
    let t = m.active_mut();
    if t.js.is_some() {
        let (_, pending) = dispatch_window_js_event_on_tab(t, "beforeunload", now_ms);
        for req in pending {
            handle.dispatch(req);
        }
    }
    let referer = if t.url == NEW_TAB_URL || t.url.is_empty() { None } else { Some(t.url.clone()) };
    t.url = url.clone();
    t.addr.set_text(url.clone());
    t.addr_focused = false;
    t.status = format!("POST {url}…");
    t.scroll_y = 0.0;
    t.box_tree = None;
    t.history.truncate(t.cursor + 1);
    if t.history.last() != Some(&url) {
        t.history.push(url.clone());
        t.cursor = t.history.len() - 1;
    }
    t.gen = t.gen.wrapping_add(1);
    let (id, gen) = (t.id, t.gen);
    let h = handle.clone();
    std::thread::spawn(move || {
        let engine = Engine::new();
        match engine.load_post_with_referer(&url, &body, referer.as_deref()) {
            Ok(doc) => {
                let title = if doc.title.is_empty() { doc.url.clone() } else { doc.title.clone() };
                h.dispatch(Msg::Loaded {
                    tab: id,
                    gen,
                    final_url: doc.url.clone(),
                    title,
                    box_tree: doc.box_tree,
                    source: doc.source,
                    meta_refresh: doc.meta_refresh,
                    scripts: doc.scripts,
                });
            }
            Err(e) => h.dispatch(Msg::LoadFailed { tab: id, gen, err: e.to_string() }),
        }
    });
}

/// Decide qué nombre usar para una descarga. El hint del attr
/// `download="..."` gana si no está vacío y no contiene `/` (un attr
/// `download="../etc/passwd"` debe rechazarse — es vector de path
/// traversal). Sino, usamos el último segmento del path de la URL; si
/// la URL no tiene path significativo, fallback a `descarga`.
pub(crate) fn pick_download_filename(url: &str, hint: &str) -> String {
    let hint = hint.trim();
    if !hint.is_empty() && !hint.contains('/') && !hint.contains('\\') {
        return hint.to_string();
    }
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(seg) = parsed.path_segments().and_then(|s| s.last()) {
            let seg = seg.trim();
            if !seg.is_empty() {
                return seg.to_string();
            }
        }
    }
    "descarga".to_string()
}

/// Path absoluto donde la descarga termina. Convención: `$XDG_DOWNLOAD_DIR/
/// puriy/<filename>` o, sin xdg, `~/Downloads/puriy/<filename>`. Si
/// ningún path conocido es accesible, cae a `/tmp/`.
pub(crate) fn download_path(filename: &str) -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DOWNLOAD_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Downloads"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    base.join("puriy").join(filename)
}
