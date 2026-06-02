//! Puente JS ↔ chrome: corre los scripts del documento en el `JsRuntime` de la
//! pestaña, sincroniza media queries, recolecta snapshots de elementos para el
//! DOM espejo, despacha eventos (click/key/window/document) al runtime, tickea
//! los runtimes (timers/rAF) y aplica de vuelta al `BoxTree`/estado las
//! mutaciones que el JS produjo (`apply_dom_mutations`, incl. refresco de
//! canvas). Extraído de `lib.rs` (regla #1). Comparte los tipos del crate vía
//! `use super::*`.
use super::*;

/// Ejecuta los scripts inline del documento en el `JsRuntime` de la
/// pestaña. Crea el runtime lazily si no existía. Llama `set_document`
/// con un snapshot (`title`, `url`, `body_text`) para que `document.*`
/// devuelva valores reales en lugar de undefined.
///
/// Scripts externos (`src=`) llegan acá ya descargados por
/// `puriy_engine::scripts::fetch_externals` (Fase 7.4): el body UTF-8
/// quedó copiado en `inline`. Si la descarga falló, `inline` sigue en
/// `None` y se saltea silenciosamente (no es error JS — es network).
/// Scripts `is_module=true` se saltean: el runtime de Fase 7.x es
/// clásico (no module loader).
///
/// `t.js_summary` se actualiza con counts agregados. La función NO toca
/// `t.status` — el caller decide cómo mostrarlo.
pub(crate) fn run_scripts_on_tab(
    t: &mut TabState,
    scripts: &[puriy_engine::ScriptInfo],
    now_ms: u64,
    system_clipboard: Option<&str>,
) -> Vec<Msg> {
    if scripts.is_empty() {
        return Vec::new();
    }
    // Body text — concatenación de las hojas de texto del box tree.
    // Snapshot a momento de Load; muta si la página re-renderiza pero
    // el JS no re-lee. Fase 7.5+ lo hará reactivo.
    let body_text = t
        .box_tree
        .as_ref()
        .map(extract_body_text)
        .unwrap_or_default();
    // Lazy: instanciar el JsRuntime cuesta ~200ms — sólo si la página
    // realmente tiene scripts ejecutables.
    let has_executable = scripts
        .iter()
        .any(|s| s.inline.is_some() && !s.is_module);
    if !has_executable {
        return Vec::new();
    }
    let rt = match puriy_js::JsRuntime::new() {
        Ok(r) => Box::new(r),
        Err(_) => {
            t.js_summary.errors += 1;
            return Vec::new();
        }
    };
    // Snapshot de elementos con `id=` — el harness JS los expone via
    // `getElementById` / `document.querySelector('#x')`. textContent
    // del subárbol de cada uno (snapshot read-only, igual que body).
    let elements = t
        .box_tree
        .as_ref()
        .map(collect_element_snapshots)
        .unwrap_or_default();
    t.js = Some(rt);
    let rt = t.js.as_mut().unwrap();
    let _ = rt.set_document(&t.title, &t.url, &body_text);
    let _ = rt.set_elements(&elements);
    // Reloj inicial — sin esto, `setTimeout(fn, 100)` registrado por un
    // script inicial dispararía contra `__puriy_now_ms=0` y se vencería
    // en el primer tick que cruce 100ms del wall clock (raro pero
    // posible). Setearlo acá los ancla al reloj real del chrome.
    let _ = rt.set_now_ms(now_ms);
    // Fase 7.28 — sync scroll + viewport. Habilita que `window.scrollY`/
    // `innerWidth` desde JS reflejen state real del chrome. El viewport
    // sale del thread-local `PURIY_VIEWPORT`, que `Msg::Resize` mantiene
    // al día con el tamaño real de la ventana (default = initial_size).
    let (vp_w, vp_h) = PURIY_VIEWPORT.with(|c| c.get());
    let _ = rt.set_scroll(0.0, t.scroll_y);
    let _ = rt.set_viewport(vp_w, vp_h);
    // DPR real de la ventana (Fase 7.173): que `devicePixelRatio` sea
    // correcto ya en el primer script. `Msg::ScaleFactor` mantiene el
    // thread-local al día con el `scale_factor` de winit (default = 1.0).
    let _ = rt.set_device_pixel_ratio(PURIY_DPR.with(|c| c.get()));
    // Portapapeles del sistema → buffer JS (Fase 7.176): que un
    // `navigator.clipboard.readText()` de un script inicial vea lo que el
    // usuario tiene copiado afuera, no la cadena vacía. (Limitación: un copy
    // externo POSTERIOR al load no se relee hasta la próxima carga — la lectura
    // viva exigiría resolver readText como promesa pendiente del chrome.)
    if let Some(clip) = system_clipboard {
        let _ = rt.set_clipboard(clip);
    }
    let mut prev_stdout_len = rt.stdout().len();
    let mut prev_stderr_len = rt.stderr().len();
    for s in scripts {
        if s.is_module {
            continue;
        }
        let Some(body) = s.inline.as_ref() else {
            continue;
        };
        // Skip non-JS types (templates, application/json, etc.).
        if let Some(t_attr) = &s.type_attr {
            let l = t_attr.to_ascii_lowercase();
            if !l.is_empty()
                && !l.contains("javascript")
                && !l.contains("ecmascript")
                && l != "text/js"
            {
                continue;
            }
        }
        if let Err(_e) = rt.eval(body) {
            t.js_summary.errors += 1;
        }
        // Contá líneas nuevas en stdout/stderr — `console.log` agrega
        // exactamente una `\n` por llamada.
        let new_stdout = rt.stdout();
        let new_stderr = rt.stderr();
        t.js_summary.logs += new_stdout[prev_stdout_len..].matches('\n').count();
        t.js_summary.errors += new_stderr[prev_stderr_len..].matches('\n').count();
        prev_stdout_len = new_stdout.len();
        prev_stderr_len = new_stderr.len();
    }
    // Resuelve las media queries que los scripts consultaron (`matchMedia`)
    // contra el viewport real, ahora que ya se registraron. Así un listener
    // de DOMContentLoaded/load o un `if (mql.matches)` posterior ve el valor
    // correcto. (Limitación: una lectura síncrona de `.matches` en el MISMO
    // tick del `matchMedia(...)` ve aún `false` — no hay hostcall síncrono
    // desde el sandbox para evaluar al vuelo.)
    let (vp_w, vp_h) = PURIY_VIEWPORT.with(|c| c.get());
    sync_media_queries(rt, vp_w, vp_h, PURIY_DPR.with(|c| c.get()) as f32);
    // Aplica al box_tree cualquier mutación que los scripts iniciales
    // hayan hecho via `el.textContent = ...` (typeahead, contadores
    // inicializados, sustituciones de placeholders, etc). Las
    // mutaciones de fetch suben al caller para que dispatch.
    apply_dom_mutations(t)
}

/// Evalúa cada media query registrada por `matchMedia` contra el viewport
/// real (ancho/alto en px + DPR) reusando el evaluador del engine, y empuja
/// el resultado al estado JS — disparando `change` en los `MediaQueryList`
/// vivos cuyo `matches` flipeó. No-op si el script nunca llamó `matchMedia`.
pub(crate) fn sync_media_queries(rt: &mut puriy_js::JsRuntime, vp_w: f32, vp_h: f32, dpr: f32) {
    let queries = rt.registered_media_queries();
    if queries.is_empty() {
        return;
    }
    let vp = puriy_engine::Viewport { width: vp_w, height: vp_h, dpr };
    for q in queries {
        let matches = puriy_engine::evaluate_media_query(&q, vp);
        let _ = rt.set_media_match(&q, matches);
    }
}

/// Walka el `BoxTree` y arma un `Vec<ElementSnapshot>` para cada nodo
/// con `element_id` no-vacío. El `text_content` del snapshot es la
/// concatenación de las hojas de texto del subárbol (con separadores
/// espacio), análoga a `body.textContent` pero scoped al elemento.
///
/// Sólo nodos con `id=` se exponen — match exacto del modelo que el
/// harness JS usa (índice `__puriy_elements[id]`). Elementos sin id no
/// se exponen ni a `getElementById` ni a event handlers.
pub(crate) fn collect_element_snapshots(bt: &BoxTree) -> Vec<puriy_js::ElementSnapshot> {
    let mut out = Vec::new();
    // Fase 7.10 — walk recursivo manual para que cada elemento conozca
    // el id de su ancestro Element más cercano con id=. `bt.walk(|b|)`
    // no propaga contexto del parent, así que usamos rec con stack.
    // Fase 7.29 — además contamos DFS pre-order para `dfs_index`, que
    // alimenta `getBoundingClientRect` heurístico (top = (idx-1) × 30).
    fn rec(
        b: &BoxNode,
        parent_id: Option<&str>,
        counter: &mut u32,
        out: &mut Vec<puriy_js::ElementSnapshot>,
    ) {
        *counter += 1;
        let my_dfs = *counter;
        let my_id_opt = b.element_id.as_deref().filter(|s| !s.is_empty());
        if let Some(id) = my_id_opt {
            let tag_name = b.tag.clone().unwrap_or_default();
            let text_content = node_text_content(b);
            let value = if b.input_kind.is_some() {
                b.input_initial.clone().or_else(|| Some(String::new()))
            } else if let Some(sel) = &b.select {
                sel.options
                    .get(sel.initial)
                    .map(|opt| opt.value.clone())
                    .or_else(|| Some(String::new()))
            } else {
                None
            };
            let dataset = b
                .dataset()
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            out.push(puriy_js::ElementSnapshot {
                id: id.to_string(),
                tag_name,
                text_content,
                class_list: b.class_list.clone(),
                value,
                parent_id: parent_id.map(String::from),
                dataset,
                attributes: b.attributes.clone(),
                dfs_index: my_dfs,
            });
        }
        let next_parent = my_id_opt.or(parent_id);
        for c in &b.children {
            rec(c, next_parent, counter, out);
        }
    }
    let mut counter: u32 = 0;
    rec(&bt.root, None, &mut counter, &mut out);
    out
}

/// Concatena las hojas de texto del subárbol del nodo `b`, separadas
/// por espacio. Mismo molde que `extract_body_text` pero scoped — útil
/// para que `el.textContent` devuelva sólo lo que vive bajo el elemento.
pub(crate) fn node_text_content(b: &BoxNode) -> String {
    let mut out = String::new();
    fn rec(b: &BoxNode, out: &mut String) {
        if let Some(text) = &b.text {
            if !text.is_empty() {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(text);
            }
        }
        for c in &b.children {
            rec(c, out);
        }
    }
    rec(b, &mut out);
    out
}

/// Dispara los handlers JS registrados sobre `element_id` (vía
/// `onclick` / `addEventListener`) en la pestaña activa. Si el runtime
/// no existe o ningún handler corrió, queda como no-op — el chrome
/// no aplica fallback al default action (los `<a>` con id+link ya
/// navegan por el path nativo, este msg sólo se arma para elementos
/// sin link).
pub(crate) fn dispatch_js_event(
    m: &mut Model,
    element_id: &str,
    event_type: &str,
    now_ms: u64,
) -> (puriy_js::DispatchResult, Vec<Msg>) {
    dispatch_js_event_with_init(m, element_id, event_type, now_ms, None)
}

/// Fase 7.9 — variante con `EventInit` opcional. El init lleva los
/// campos enriquecidos del DOM Event (key/code/modifiers para keydown,
/// value para change/input). Si es `None`, el handler recibe el event
/// "viejo" estilo Fase 7.6 (type/target/preventDefault) — backwards
/// compatible.
pub(crate) fn dispatch_js_event_with_init(
    m: &mut Model,
    element_id: &str,
    event_type: &str,
    now_ms: u64,
    init: Option<puriy_js::EventInit>,
) -> (puriy_js::DispatchResult, Vec<Msg>) {
    let t = m.active_mut();
    let Some(rt) = t.js.as_mut() else {
        return (puriy_js::DispatchResult::default(), Vec::new());
    };
    // Fase 7.11 — refresh del fuel antes de cada dispatch. Cada evento
    // de usuario (click/keydown/focus/blur/change/input) es una unidad
    // independiente: que un dispatch anterior haya consumido fuel no
    // debe limitar al siguiente. El cap por evento sigue siendo
    // DEFAULT_FUEL (50M) — corta loops infinitos dentro de un handler.
    rt.set_fuel(puriy_js::DEFAULT_FUEL);
    let _ = rt.set_now_ms(now_ms);
    // Fase 7.28 — refresh scroll antes del dispatch: el handler puede
    // leer `window.scrollY` para "estoy en el footer?" o "header
    // sticky?". Sin esto, leería el último valor que el JS mismo
    // escribió, no el scroll real del usuario.
    let _ = rt.set_scroll(0.0, t.scroll_y);
    let prev_stdout_len = rt.stdout().len();
    let prev_stderr_len = rt.stderr().len();
    let mut result = match rt.dispatch_event(element_id, event_type, init.as_ref()) {
        Ok(r) => {
            let new_stdout = rt.stdout();
            let new_stderr = rt.stderr();
            t.js_summary.logs += new_stdout[prev_stdout_len..].matches('\n').count();
            t.js_summary.errors += new_stderr[prev_stderr_len..].matches('\n').count();
            r
        }
        Err(_) => {
            t.js_summary.errors += 1;
            puriy_js::DispatchResult::default()
        }
    };
    let mut pending = apply_dom_mutations(t);
    // Bubbling a `document` (event delegation): tras correr los handlers del
    // elemento, los eventos que bubblean también disparan los listeners de
    // `document.addEventListener(type, ...)`, con el elemento original como
    // `event.target`. Un `preventDefault()` del handler de document también
    // cuenta para el fallback (p. ej. cancelar la navegación de un `<a>`).
    // Si un handler del elemento llamó `stopPropagation()`, el evento NO
    // burbujea hasta `document` (el dispatch ahora propaga el flag
    // `_stopped` vía `DispatchResult::propagation_stopped`).
    if event_bubbles_to_document(event_type) && !result.propagation_stopped {
        let (doc_result, doc_pending) = dispatch_document_js_event_on_tab(
            t,
            event_type,
            init.as_ref(),
            Some(element_id),
            now_ms,
        );
        result.count += doc_result.count;
        result.default_prevented |= doc_result.default_prevented;
        pending.extend(doc_pending);
    }
    (result, pending)
}

/// ¿Este tipo de evento bubblea hasta `document`? Cubre los eventos que la
/// gente delega con `document.addEventListener` (click, teclas, input/change,
/// submit). `focus`/`blur` quedan afuera a propósito: en spec NO bubblean
/// (sus variantes `focusin`/`focusout` sí, pero el chrome no las emite aún).
pub(crate) fn event_bubbles_to_document(event_type: &str) -> bool {
    matches!(
        event_type,
        "click"
            | "dblclick"
            | "mousedown"
            | "mouseup"
            | "keydown"
            | "keyup"
            | "keypress"
            | "input"
            | "change"
            | "submit"
    )
}

/// Fase 7.42 — cambia la pestaña activa, marcando la vieja como hidden y
/// la nueva como visible (dispatcha `'visibilitychange'` en cada una vía
/// `set_visibility`). Apps que pausan video / polling / animation al
/// background ven el evento sin necesidad de cabling especial en el msg.
pub(crate) fn switch_active_tab(m: &mut Model, new_idx: usize) {
    let prev_idx = m.active;
    if prev_idx == new_idx {
        return;
    }
    if let Some(rt) = m.tabs[prev_idx].js.as_mut() {
        let _ = rt.set_visibility(true);
    }
    m.active = new_idx;
    if let Some(rt) = m.tabs[new_idx].js.as_mut() {
        let _ = rt.set_visibility(false);
    }
}

/// Fase 7.39 — dispatcha un evento sobre `window` (no sobre un elemento)
/// para una pestaña dada. Refresca fuel/now/scroll antes para que los
/// handlers vean state consistente y dropea mutaciones DOM resultantes
/// en el return (igual que `dispatch_js_event`). Toma `&mut TabState`
/// directo (no `Model`) para que la pestaña pueda no ser la activa —
/// 'load' puede dispararse en background loads.
pub(crate) fn dispatch_window_js_event_on_tab(
    t: &mut TabState,
    event_type: &str,
    now_ms: u64,
) -> (puriy_js::DispatchResult, Vec<Msg>) {
    let Some(rt) = t.js.as_mut() else {
        return (puriy_js::DispatchResult::default(), Vec::new());
    };
    rt.set_fuel(puriy_js::DEFAULT_FUEL);
    let _ = rt.set_now_ms(now_ms);
    let _ = rt.set_scroll(0.0, t.scroll_y);
    let prev_stdout_len = rt.stdout().len();
    let prev_stderr_len = rt.stderr().len();
    let result = match rt.dispatch_window_event(event_type, None) {
        Ok(r) => {
            let new_stdout = rt.stdout();
            let new_stderr = rt.stderr();
            t.js_summary.logs += new_stdout[prev_stdout_len..].matches('\n').count();
            t.js_summary.errors += new_stderr[prev_stderr_len..].matches('\n').count();
            r
        }
        Err(_) => {
            t.js_summary.errors += 1;
            puriy_js::DispatchResult::default()
        }
    };
    let pending = apply_dom_mutations(t);
    (result, pending)
}

/// Dispatcha un evento a nivel `document` (`document.addEventListener`).
/// Cubre `DOMContentLoaded` (sin target) y la fase de delegación de eventos
/// de elemento (`target_element_id` = el elemento original que bubblea hasta
/// `document`). Espejo de [`dispatch_window_js_event_on_tab`]: contabiliza
/// logs/errores y drena las mutaciones DOM que el handler haya producido.
pub(crate) fn dispatch_document_js_event_on_tab(
    t: &mut TabState,
    event_type: &str,
    init: Option<&puriy_js::EventInit>,
    target_element_id: Option<&str>,
    now_ms: u64,
) -> (puriy_js::DispatchResult, Vec<Msg>) {
    let Some(rt) = t.js.as_mut() else {
        return (puriy_js::DispatchResult::default(), Vec::new());
    };
    rt.set_fuel(puriy_js::DEFAULT_FUEL);
    let _ = rt.set_now_ms(now_ms);
    let _ = rt.set_scroll(0.0, t.scroll_y);
    let prev_stdout_len = rt.stdout().len();
    let prev_stderr_len = rt.stderr().len();
    let result = match rt.dispatch_document_event(event_type, init, target_element_id) {
        Ok(r) => {
            let new_stdout = rt.stdout();
            let new_stderr = rt.stderr();
            t.js_summary.logs += new_stdout[prev_stdout_len..].matches('\n').count();
            t.js_summary.errors += new_stderr[prev_stderr_len..].matches('\n').count();
            r
        }
        Err(_) => {
            t.js_summary.errors += 1;
            puriy_js::DispatchResult::default()
        }
    };
    let pending = apply_dom_mutations(t);
    (result, pending)
}

/// Mapea un `KeyEvent` de Llimphi a un `EventInit` enriquecido con los
/// campos estándar del DOM keydown/keyup event.
///
/// - `event.key`: el "key value" — `e.text` cuando hay carácter
///   imprimible (respeta modifiers + IME), o el nombre del NamedKey
///   (`"Enter"`, `"ArrowLeft"`, etc.) para teclas no-imprimibles.
/// - `event.code`: el "physical code". Llimphi no expone el código
///   físico (winit lo tiene en `KeyEvent.physical_key` que no propagamos);
///   por ahora replicamos `key` como aproximación. Suficiente para que
///   handlers que filtran `if (e.code === 'Enter')` funcionen.
/// - `event.shiftKey`/`ctrlKey`/`altKey`/`metaKey`: directos de los
///   modifiers.
pub(crate) fn key_event_to_init(e: &llimphi_ui::KeyEvent) -> puriy_js::EventInit {
    let key = match &e.key {
        llimphi_ui::Key::Character(s) => s.to_string(),
        llimphi_ui::Key::Named(n) => named_key_name(n),
        _ => e.text.clone().unwrap_or_default(),
    };
    puriy_js::EventInit {
        key: Some(key.clone()),
        code: Some(key),
        shift_key: Some(e.modifiers.shift),
        ctrl_key: Some(e.modifiers.ctrl),
        alt_key: Some(e.modifiers.alt),
        meta_key: Some(e.modifiers.meta),
        value: None,
    }
}

/// Nombre canónico de un `NamedKey` al estilo DOM (`"Enter"`,
/// `"ArrowLeft"`, `"Escape"`, etc.). Cubre las teclas que un browser
/// típico usa para keydown handlers. Para teclas no mapeadas, usa
/// `{:?}` de Debug — degrada limpio sin perder información.
pub(crate) fn named_key_name(n: &llimphi_ui::NamedKey) -> String {
    use llimphi_ui::NamedKey;
    match n {
        NamedKey::Enter => "Enter".into(),
        NamedKey::Escape => "Escape".into(),
        NamedKey::Tab => "Tab".into(),
        NamedKey::Backspace => "Backspace".into(),
        NamedKey::Delete => "Delete".into(),
        NamedKey::Space => " ".into(),
        NamedKey::ArrowLeft => "ArrowLeft".into(),
        NamedKey::ArrowRight => "ArrowRight".into(),
        NamedKey::ArrowUp => "ArrowUp".into(),
        NamedKey::ArrowDown => "ArrowDown".into(),
        NamedKey::Home => "Home".into(),
        NamedKey::End => "End".into(),
        NamedKey::PageUp => "PageUp".into(),
        NamedKey::PageDown => "PageDown".into(),
        NamedKey::Shift => "Shift".into(),
        NamedKey::Control => "Control".into(),
        NamedKey::Alt => "Alt".into(),
        NamedKey::Meta => "Meta".into(),
        NamedKey::CapsLock => "CapsLock".into(),
        NamedKey::F1 => "F1".into(),
        NamedKey::F2 => "F2".into(),
        NamedKey::F3 => "F3".into(),
        NamedKey::F4 => "F4".into(),
        NamedKey::F5 => "F5".into(),
        NamedKey::F6 => "F6".into(),
        NamedKey::F7 => "F7".into(),
        NamedKey::F8 => "F8".into(),
        NamedKey::F9 => "F9".into(),
        NamedKey::F10 => "F10".into(),
        NamedKey::F11 => "F11".into(),
        NamedKey::F12 => "F12".into(),
        other => format!("{:?}", other),
    }
}

/// Avanza el reloj de cada `JsRuntime` vivo del Model al `now_ms` actual
/// y dispara los callbacks `setTimeout`/`setInterval` vencidos. Llamado
/// desde `Msg::JsTick` (cada `JS_POLL_PERIOD_MS`).
///
/// Pestañas sin runtime se saltean en ~ns (chequeo `Option::is_some`).
/// Pestañas con runtime pero sin timers vivos también se saltean tras
/// un `pending_timers` que cuesta un eval mini (~µs). No queremos
/// dejar de polear porque mismo runtime puede registrar timers más
/// tarde via event handlers (Fase 7.5b+).
///
/// Cada disparo nuevo de stdout/stderr se cuenta a `t.js_summary`,
/// alineado con el conteo que hace `run_scripts_on_tab`.
pub(crate) fn tick_js_runtimes(m: &mut Model, now_ms: u64) -> Vec<Msg> {
    let mut pending: Vec<Msg> = Vec::new();
    for t in m.tabs.iter_mut() {
        let Some(rt) = t.js.as_mut() else { continue };
        if rt.pending_timers() == 0 {
            continue;
        }
        // Fase 7.11 — refresh del fuel por tick. Cada tick es una unidad
        // independiente al estilo del event loop; no acumulamos cap.
        rt.set_fuel(puriy_js::DEFAULT_FUEL);
        // Fase 7.28 — scroll sync para los rAF/setInterval callbacks que
        // leen window.scrollY (animation loops chequeando posición).
        let _ = rt.set_scroll(0.0, t.scroll_y);
        let prev_stdout_len = rt.stdout().len();
        let prev_stderr_len = rt.stderr().len();
        match rt.tick(now_ms) {
            Ok(_r) => {
                let new_stdout = rt.stdout();
                let new_stderr = rt.stderr();
                t.js_summary.logs += new_stdout[prev_stdout_len..].matches('\n').count();
                t.js_summary.errors += new_stderr[prev_stderr_len..].matches('\n').count();
            }
            Err(_) => {
                t.js_summary.errors += 1;
            }
        }
        pending.extend(apply_dom_mutations(t));
    }
    pending
}

/// Drena el buffer de mutaciones del DOM del runtime de la pestaña y
/// las aplica al `box_tree`. Llamado después de cada operación que
/// pueda haber escrito a `textContent`/`innerHTML` (run_scripts,
/// dispatch_event, tick). Si no hay mutaciones, retorna sin tocar el
/// árbol — costo: un eval mini que devuelve `''`.
///
/// Mutaciones sobre ids que no existen en el árbol se silencian (el
/// JS puede haber retenido un handle de una página anterior, o el id
/// puede haber sido renombrado por un script DOM-mutating no soportado).
/// Fase 7.31 — el return ahora es `Vec<Msg>`: lista de FetchRequest que
/// el caller debe despachar (necesitan spawn de worker thread, que sólo
/// el call site tiene cabling para hacer). Si el caller no tiene handle
/// (ej. tests), puede ignorar el Vec. El resto de mutations se aplican
/// in-place sin requerir el handle.
pub(crate) fn apply_dom_mutations(t: &mut TabState) -> Vec<Msg> {
    let mut out = Vec::new();
    // El borrow de `rt` se acota al drain para poder refrescar canvas (que
    // re-borrowa `t`) sin conflicto.
    let muts = match t.js.as_mut() {
        Some(rt) => rt.drain_dom_mutations(),
        None => return out,
    };
    // Fase 7.196 — refrescamos los frames de `<canvas>` SIEMPRE que se corra
    // JS (no sólo cuando hay mutaciones DOM): dibujar en canvas no produce
    // mutaciones. Gateado por `has_canvas` para no evaluar en páginas sin canvas.
    if t.has_canvas {
        refresh_canvas_frames(t);
    }
    if muts.is_empty() {
        return out;
    }
    // Procesamos los fetch ANTES de chequear box_tree — los fetch no
    // requieren box_tree (operan a nivel runtime). Esto también
    // habilita fetch durante el load inicial.
    let mut other_muts = Vec::with_capacity(muts.len());
    for m in muts {
        if m.kind == "fetch" {
            if let Some(req) = parse_fetch_payload(&m.value, t.id, t.gen) {
                out.push(req);
            }
        } else if m.kind == "clipboard" {
            // `writeText:<txt>` / `write:<txt>` — empuja el texto al
            // portapapeles real. No necesita box_tree (opera sobre el SO);
            // el write efectivo lo hace el update loop (tiene `&mut clipboard`).
            let text = m
                .value
                .strip_prefix("writeText:")
                .or_else(|| m.value.strip_prefix("write:"));
            if let Some(text) = text {
                out.push(Msg::SetSystemClipboard(text.to_string()));
            }
        } else if m.kind == "eventsource" {
            // EventSource: `<id> GS open GS <url> GS <withCred>` o `<id> GS close`.
            // El worker de streaming lo arranca/corta el update loop (necesita
            // handle + `&mut tab` para el flag de cancelación).
            let parts: Vec<&str> = m.value.split('\u{001D}').collect();
            if let Some(es_id) = parts.first().and_then(|s| s.parse::<u32>().ok()) {
                match parts.get(1).copied() {
                    Some("open") => {
                        if let Some(url) = parts.get(2) {
                            out.push(Msg::EsOpen {
                                tab: t.id,
                                gen: t.gen,
                                es_id,
                                url: url.to_string(),
                            });
                        }
                    }
                    Some("close") => out.push(Msg::EsClose { tab: t.id, es_id }),
                    _ => {}
                }
            }
        } else {
            other_muts.push(m);
        }
    }
    let muts = other_muts;
    if muts.is_empty() {
        return out;
    }
    let Some(bt) = t.box_tree.as_mut() else { return out };
    let mut needs_restyle = false;
    for m in muts {
        if m.kind == "text" {
            bt.set_element_text_content(&m.id, &m.value);
        } else if let Some(prop) = m.kind.strip_prefix("style:") {
            // Fase 7.8: el.style.X = Y publica con kind = "style:X" (X
            // ya viene en kebab-case desde el harness JS).
            bt.set_element_style(&m.id, prop, &m.value);
        } else if m.kind == "appendChild" {
            // Fase 7.12: el.appendChild(child) publica con kind =
            // "appendChild", value = "tag<US>id<US>text<US>classes<US>value"
            // donde <US> es U+001D (Group Separator). Construimos un
            // BoxNode sintético via synthesize_box_node + push al
            // parent.children.
            let parts: Vec<&str> = m.value.split('\u{001D}').collect();
            if parts.len() >= 5 {
                let tag = parts[0];
                let cid = parts[1];
                let text = parts[2];
                let classes: Vec<String> = parts[3]
                    .split_whitespace()
                    .filter(|p| !p.is_empty())
                    .map(|p| p.to_string())
                    .collect();
                let value = if parts[4].is_empty() { None } else { Some(parts[4]) };
                let cid_opt = if cid.is_empty() { None } else { Some(cid) };
                let child =
                    puriy_engine::synthesize_box_node(tag, cid_opt, text, classes, value);
                bt.append_child_to(&m.id, child);
            }
        } else if m.kind == "insertBefore" {
            // Fase 7.14: payload = mismo formato que appendChild más
            // un 6º campo con ref_id (el id del sibling antes del cual
            // insertar). Si ref_id no se encuentra, fallback a append.
            let parts: Vec<&str> = m.value.split('\u{001D}').collect();
            if parts.len() >= 6 {
                let tag = parts[0];
                let cid = parts[1];
                let text = parts[2];
                let classes: Vec<String> = parts[3]
                    .split_whitespace()
                    .filter(|p| !p.is_empty())
                    .map(|p| p.to_string())
                    .collect();
                let value = if parts[4].is_empty() { None } else { Some(parts[4]) };
                let ref_id = parts[5];
                let cid_opt = if cid.is_empty() { None } else { Some(cid) };
                let child =
                    puriy_engine::synthesize_box_node(tag, cid_opt, text, classes, value);
                bt.insert_child_before(&m.id, child, ref_id);
            }
        } else if m.kind == "removeChild" {
            // Fase 7.12: value = id del child (synth_id o user-set id).
            bt.remove_child_by_id(&m.id, &m.value);
        } else if let Some(key) = m.kind.strip_prefix("dataset:") {
            // Fase 7.11: el.dataset.fooBar = X publica con kind =
            // "dataset:foo-bar" (key ya viene kebab desde el harness JS).
            bt.set_element_dataset(&m.id, key, &m.value);
        } else if let Some(key) = m.kind.strip_prefix("dataset-remove:") {
            // Fase 7.11: delete el.dataset.fooBar publica con kind =
            // "dataset-remove:foo-bar".
            bt.remove_element_dataset(&m.id, key);
        } else if let Some(name) = m.kind.strip_prefix("attr:") {
            // Fase 7.16: el.setAttribute(name, value) publica con kind =
            // "attr:<name-lowercase>" para atributos no especiales
            // (aria-*, href, src, title, role, etc.). El name viene ya
            // lowercased desde el harness JS.
            bt.set_element_attribute(&m.id, name, &m.value);
        } else if let Some(name) = m.kind.strip_prefix("attr-remove:") {
            // Fase 7.16: el.removeAttribute(name) publica con kind =
            // "attr-remove:<name-lowercase>".
            bt.remove_element_attribute(&m.id, name);
        } else if m.kind == "value" {
            // Fase 7.9: el.value = X aplica al TextInputState (para
            // <input>/<textarea>) o al SelectState (para <select>).
            // Si el id matchea un input slot, set_text. Si matchea un
            // select slot, busca el option con value == X y selecciónalo.
            if let Some(slot) = t
                .inputs_element_ids
                .iter()
                .position(|e| e.as_deref() == Some(m.id.as_str()))
            {
                if let Some(input) = t.inputs.get_mut(slot) {
                    input.set_text(m.value.clone());
                }
            } else if let Some(slot) = t
                .selects_element_ids
                .iter()
                .position(|e| e.as_deref() == Some(m.id.as_str()))
            {
                if let Some(opt_idx) = select_option_index_by_value(bt, slot, &m.value) {
                    if let Some(s) = t.selects.get_mut(slot) {
                        s.selected = opt_idx;
                    }
                }
            }
        } else if m.kind == "focus" {
            // Fase 7.18: el.focus() desde JS. Si el id corresponde a un
            // input slot, mueve el cursor del usuario allí (focused_input
            // = Some(slot)). Si no es input, no-op silencioso — un
            // .focus() sobre un button/div sólo dispara el event handler.
            if let Some(slot) = t
                .inputs_element_ids
                .iter()
                .position(|e| e.as_deref() == Some(m.id.as_str()))
            {
                t.focused_input = Some(slot);
            }
        } else if m.kind == "blur" {
            // Fase 7.18: el.blur() desde JS. Sólo limpia focused_input si
            // el elemento era el actualmente focado — un .blur() sobre un
            // input no-focado no afecta el cursor del usuario.
            if let Some(slot) = t
                .inputs_element_ids
                .iter()
                .position(|e| e.as_deref() == Some(m.id.as_str()))
            {
                if t.focused_input == Some(slot) {
                    t.focused_input = None;
                }
            }
        } else if m.kind == "scroll" {
            // Fase 7.26: window.scrollTo(x, y) publica con id vacío y
            // value = "x,y". Sólo aplicamos la coord Y al scroll_y del
            // tab (no tenemos scroll horizontal por ahora).
            if let Some(comma) = m.value.find(',') {
                if let Ok(y) = m.value[comma + 1..].parse::<f32>() {
                    t.scroll_y = y.max(0.0);
                }
            }
        } else if m.kind == "scrollTop" {
            // Fase 7.26: el.scrollTop = N. Aplica sólo si el id matchea
            // body/html/main (el "viewport root" del tab). Otros
            // elementos requerirían scroll containers per-elemento.
            let mut applied = false;
            bt.walk(|b| {
                if applied {
                    return;
                }
                if b.element_id.as_deref() == Some(m.id.as_str()) {
                    let is_root =
                        matches!(b.tag.as_deref(), Some("body") | Some("html") | Some("main"));
                    if is_root {
                        if let Ok(y) = m.value.parse::<f32>() {
                            t.scroll_y = y.max(0.0);
                            applied = true;
                        }
                    }
                }
            });
            let _ = applied; // permite que se compile aunque no se use; doc-only
        } else if m.kind == "scrollLeft" {
            // Fase 7.26: scrollLeft no aplica — no tenemos scroll
            // horizontal en el chrome. No-op silencioso.
        } else if m.kind == "scrollIntoView" {
            // Fase 7.24: scroll heurístico DFS-order × 30px. Sin layout
            // taffy exacto (vive sólo en frame render), aproximamos la
            // posición del element_id contando elementos en DFS pre-order.
            // Monotónico — elementos más profundos quedan más abajo, lo
            // que matchea la intuición de "scrollIntoView".
            let mut count: u32 = 0;
            let mut found_at: Option<u32> = None;
            bt.walk(|b| {
                if found_at.is_some() {
                    return;
                }
                count += 1;
                if b.element_id.as_deref() == Some(m.id.as_str()) {
                    found_at = Some(count);
                }
            });
            if let Some(pos) = found_at {
                t.scroll_y = (pos.saturating_sub(1) as f32) * 30.0;
            }
        } else if m.kind == "classList" {
            // Fase 7.184 — classList.add/remove/toggle/className/setAttribute
            // ('class') publican la lista completa de clases. Actualizamos la
            // `class_list` del nodo y marcamos para recascadear una sola vez
            // al final (un handler puede togglear varias clases por evento).
            let classes: Vec<String> = m
                .value
                .split_whitespace()
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            if bt.set_element_class_list(&m.id, classes) {
                needs_restyle = true;
            }
        }
    }
    if needs_restyle {
        // Recascada del documento entero: un cambio de clase puede afectar
        // descendientes (selectores descendientes/herencia) y hermanos
        // posteriores (`+`/`~`). Reusa el motor de cascada del build.
        bt.restyle();
    }
    out
}
