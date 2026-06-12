use super::*;

impl App for Puriy {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "puriy · navegador soberano"
    }

    fn app_id() -> Option<&'static str> {
        Some("net.tawasuyu.puriy")
    }

    fn initial_size() -> (u32, u32) {
        (1100, 760)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let url = PURIY_URL
            .with(|c| c.borrow().clone())
            .unwrap_or_else(|| NEW_TAB_URL.to_string());
        let mut tab = TabState::new(url.clone());
        tab.gen = 1;
        spawn_load(tab.id, tab.gen, url, /* referer */ None, current_viewport(), handle.clone());
        // Poll del reactor JS — un solo thread global que dispatcha
        // `Msg::JsTick` cada ~33ms. El handler walka las pestañas y
        // saltea las que no tienen runtime (cost ~ns por tab inactiva).
        handle.spawn_periodic(
            std::time::Duration::from_millis(JS_POLL_PERIOD_MS),
            || Msg::JsTick,
        );
        // Orientación inicial desde el Profile (si está cableado); default
        // horizontal (un nivel, comportamiento clásico).
        let orientation = profile_handle()
            .and_then(|h| h.lock().ok().map(|p| p.ui.orientation.clone()))
            .and_then(|o| TabOrientation::from_id(&o))
            .unwrap_or(TabOrientation::Horizontal);
        Model {
            tabs: vec![tab],
            active: 0,
            spaces: vec![Space::new("Principal", "◆")],
            active_space: 0,
            orientation,
            theme: Theme::dark(),
            settings_open: false,
            settings: AllichayState::new(),
            addr_suggest: Vec::new(),
            zoom: 1.0,
            find_active: false,
            find_input: TextInputState::new(),
            find_current: 0,
            find_case_sensitive: false,
            find_whole_word: false,
            panel: None,
            panel_filter: TextInputState::new(),
            hover_link: None,
            start: std::time::Instant::now(),
            menu_open: None,
            edit_menu: None,
            clipboard: SystemClipboard::new(),
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
        }
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Panel de configuración abierto: Esc lo cierra; el resto de teclas se
        // tragan (los campos son dropdowns, sin edición de texto). Prioridad
        // sobre todo lo demás para que las teclas no fuguen a la página.
        if model.settings_open {
            if matches!(&e.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::CloseSettings);
            }
            return None;
        }
        // Menú principal abierto: las flechas navegan. ←/→ cambian de menú
        // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc
        // cierra. Tiene prioridad sobre todo lo demás.
        if let Some(mi) = model.menu_open {
            let n = app_menu(model).menus.len().max(1);
            match &e.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => {
                    return Some(Msg::MenuOpen(Some((mi + n - 1) % n)));
                }
                Key::Named(NamedKey::ArrowRight) => {
                    return Some(Msg::MenuOpen(Some((mi + 1) % n)));
                }
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => return Some(Msg::MenuActivate),
                _ => return None,
            }
        }
        // Menú de edición abierto: ↑/↓ navegan, Enter ejecuta, Esc cierra.
        if model.edit_menu.is_some() {
            match &e.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::EditNav(1)),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::EditNav(-1)),
                Key::Named(NamedKey::Enter) => return Some(Msg::EditActivate),
                _ => return None,
            }
        }
        let mods = e.modifiers;
        // Atajos con Ctrl — toman precedencia incluso sobre el address bar.
        if mods.ctrl {
            match &e.key {
                Key::Character(s) if s.eq_ignore_ascii_case("t") => return Some(Msg::NewTab),
                Key::Character(s) if s.eq_ignore_ascii_case("w") => {
                    return Some(Msg::CloseTab(model.active));
                }
                Key::Character(s) if s.eq_ignore_ascii_case("d") => return Some(Msg::Bookmark),
                Key::Character(s) if s.eq_ignore_ascii_case("f") => return Some(Msg::FindOpen),
                Key::Character(s) if s.eq_ignore_ascii_case("b") => {
                    return Some(Msg::ToggleBookmarks);
                }
                Key::Character(s) if s.eq_ignore_ascii_case("h") => {
                    return Some(Msg::ToggleHistory);
                }
                Key::Character(s) if s.eq_ignore_ascii_case("u") => {
                    return Some(Msg::ViewSource);
                }
                // Ctrl+, — abre/cierra el panel de configuración embebido.
                Key::Character(s) if s.as_str() == "," => {
                    return Some(if model.settings_open {
                        Msg::CloseSettings
                    } else {
                        Msg::OpenSettings
                    });
                }
                Key::Named(NamedKey::Tab) if mods.shift => return Some(Msg::PrevTab),
                Key::Named(NamedKey::Tab) => return Some(Msg::NextTab),
                // Zoom: Ctrl+= / Ctrl++ / Ctrl+- / Ctrl+0. El charset depende
                // del layout — aceptamos `=`/`+` para zoom in y `-`/`_` para
                // zoom out por compat con teclados sin numpad.
                Key::Character(s) if s.as_str() == "=" || s.as_str() == "+" => {
                    return Some(Msg::ZoomIn);
                }
                Key::Character(s) if s.as_str() == "-" || s.as_str() == "_" => {
                    return Some(Msg::ZoomOut);
                }
                Key::Character(s) if s.as_str() == "0" => return Some(Msg::ZoomReset),
                _ => {}
            }
        }
        if mods.alt {
            match &e.key {
                Key::Named(NamedKey::ArrowLeft) => return Some(Msg::Back),
                Key::Named(NamedKey::ArrowRight) => return Some(Msg::Forward),
                _ => {}
            }
        }
        // Si la find bar está activa, intercepta Esc (cerrar), Enter
        // (avanza match) y Shift+Enter (retrocede), y redirige el resto
        // al input. Tiene prioridad sobre el address bar.
        if model.find_active {
            if matches!(&e.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::FindClose);
            }
            if matches!(&e.key, Key::Named(NamedKey::Enter)) {
                return Some(if mods.shift { Msg::FindPrev } else { Msg::FindNext });
            }
            return Some(Msg::FindKey(e.clone()));
        }
        // Panel abierto (bookmarks/history): Esc cierra; resto va al
        // input del filtro. F5 no se intercepta (es semánticamente la
        // pestaña activa, no el panel).
        if model.panel.is_some() {
            if matches!(&e.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::ClosePanel);
            }
            if !matches!(&e.key, Key::Named(NamedKey::F5)) {
                return Some(Msg::PanelFilterKey(e.clone()));
            }
        }
        // Si la address bar tiene foco, redirige las teclas al input.
        if model.active().addr_focused && !matches!(&e.key, Key::Named(NamedKey::F5)) {
            return Some(Msg::AddrKey(e.clone()));
        }
        // Si hay un input/textarea del documento focado, las teclas van
        // ahí. Esc blurea (foco vuelve a la página). F5 se respeta como
        // recargar para no perder un atajo crítico. Tab/Shift+Tab cicla
        // entre inputs sin pisar el typing.
        if model.active().focused_input.is_some() {
            if matches!(&e.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::FocusInput(usize::MAX)); // sentinel = blur
            }
            if matches!(&e.key, Key::Named(NamedKey::Tab)) {
                return Some(if mods.shift { Msg::FocusPrev } else { Msg::FocusNext });
            }
            if !matches!(&e.key, Key::Named(NamedKey::F5)) {
                return Some(Msg::InputKey(e.clone()));
            }
        }
        match &e.key {
            Key::Named(NamedKey::F5) => Some(Msg::Reload),
            Key::Named(NamedKey::PageDown) => Some(Msg::Scroll(LINE_PX * 12.0)),
            Key::Named(NamedKey::PageUp) => Some(Msg::Scroll(-LINE_PX * 12.0)),
            Key::Named(NamedKey::ArrowDown) => Some(Msg::Scroll(LINE_PX)),
            Key::Named(NamedKey::ArrowUp) => Some(Msg::Scroll(-LINE_PX)),
            Key::Named(NamedKey::Home) => Some(Msg::Scroll(-1.0e9)),
            Key::Named(NamedKey::End) => Some(Msg::Scroll(1.0e9)),
            _ => None,
        }
    }

    fn on_wheel(
        _model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        mods: Modifiers,
    ) -> Option<Self::Msg> {
        wheel_to_msg(delta, mods)
    }

    fn on_resize(_model: &Self::Model, width: u32, height: u32) -> Option<Self::Msg> {
        Some(Msg::Resize(width, height))
    }

    fn on_scale_factor(_model: &Self::Model, scale: f64) -> Option<Self::Msg> {
        Some(Msg::ScaleFactor(scale))
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Reload => {
                let url = m.active().url.clone();
                start_load(&mut m, url, /* push_history */ false, handle);
            }
            Msg::Loaded { tab, gen, final_url, title, box_tree, source, meta_refresh, scripts } => {
                if let Some(idx) = m.tab_idx(tab) {
                    // Lee el portapapeles del sistema ANTES de tomar `&mut t`
                    // (borrow disjunto) para sembrarlo en el runtime nuevo.
                    let sys_clipboard = m.clipboard.get();
                    let t = &mut m.tabs[idx];
                    if t.gen == gen {
                        // Si hubo redirect, propaga la URL final a la
                        // tab, la address bar y reemplaza el slot
                        // actual de history (no empuja uno nuevo — el
                        // back debe saltar a la página *anterior* a la
                        // que pidió el redirect, no al request fallido).
                        if final_url != t.url {
                            t.url = final_url.clone();
                            t.addr.set_text(final_url.clone());
                            if let Some(slot) = t.history.get_mut(t.cursor) {
                                *slot = final_url.clone();
                            }
                        }
                        t.title = title.clone();
                        let n = box_tree.descendants_count();
                        t.status = format!("OK · {n} boxes");
                        t.source = Some(source);
                        // Prefill el estado de los <details> walkeando el
                        // árbol nuevo en orden DFS — cada `<details>`
                        // aporta un bool inicializado desde su
                        // `open` attribute.
                        let mut details_open = Vec::new();
                        let mut inputs: Vec<TextInputState> = Vec::new();
                        let mut input_checks: Vec<bool> = Vec::new();
                        let mut selects: Vec<SelectState> = Vec::new();
                        let mut inputs_element_ids: Vec<Option<String>> = Vec::new();
                        let mut selects_element_ids: Vec<Option<String>> = Vec::new();
                        let mut autofocus_idx: Option<usize> = None;
                        box_tree.walk(|b| {
                            if b.tag.as_deref() == Some("details") {
                                details_open.push(b.details_open_attr);
                            }
                            if b.input_kind.is_some() {
                                let mut s = TextInputState::new();
                                if let Some(initial) = &b.input_initial {
                                    s.set_text(initial.clone());
                                }
                                let idx = inputs.len();
                                inputs.push(s);
                                input_checks.push(b.input_checked_initial);
                                inputs_element_ids.push(b.element_id.clone());
                                if b.input_autofocus && autofocus_idx.is_none() {
                                    autofocus_idx = Some(idx);
                                }
                            }
                            if let Some(sel) = &b.select {
                                selects.push(SelectState {
                                    selected: sel.initial,
                                    open: false,
                                });
                                selects_element_ids.push(b.element_id.clone());
                            }
                        });
                        t.details_open = details_open;
                        t.inputs = inputs;
                        t.input_checks = input_checks;
                        t.selects = selects;
                        t.inputs_element_ids = inputs_element_ids;
                        t.selects_element_ids = selects_element_ids;
                        t.focused_input = autofocus_idx;
                        // Árbol nuevo → los node_id viejos ya no aplican.
                        t.hover_tweens.clear();
                        // El runtime previo se va a destruir: cortá sus
                        // EventSource (sus workers de streaming) para no fugar
                        // threads ni reinyectar al runtime viejo (Fase 7.182).
                        t.cancel_all_eventsources();
                        // Fase 7.196 — ¿hay algún `<canvas>` en el árbol? Gatea
                        // el refresh de frames (evita un `eval` por tick en
                        // páginas sin canvas). Reset de frames stale del load previo.
                        t.canvas_frames.clear();
                        t.canvas_images.clear();
                        let mut has_canvas = false;
                        box_tree.walk(|b| {
                            if b.canvas.is_some() {
                                has_canvas = true;
                            }
                        });
                        t.has_canvas = has_canvas;
                        t.box_tree = Some(box_tree);
                        // Ancla el reloj de animaciones CSS de esta carga.
                        t.anim_start_ms = m.start.elapsed().as_millis() as u64;
                        // Ejecuta los `<script>` inline del documento.
                        // Destruimos cualquier JsRuntime previo (var x = ...
                        // de la página anterior no debe fugar). Si esta
                        // página tiene scripts, instanciamos un runtime
                        // fresh, hacemos set_document con el snapshot del
                        // DOM, y eval cada script en orden. Logs y errores
                        // se acumulan en t.js_summary y se muestran en la
                        // status bar.
                        t.js = None;
                        t.js_summary = JsSummary::default();
                        let now_ms = m.start.elapsed().as_millis() as u64;
                        let pending =
                            run_scripts_on_tab(t, &scripts, now_ms, sys_clipboard.as_deref());
                        for req in pending {
                            handle.dispatch(req);
                        }
                        // 'DOMContentLoaded' al document: dispara cuando el DOM
                        // quedó parseado y los scripts inline corrieron, ANTES
                        // del 'load' del window (que en spec espera recursos).
                        // Es el evento más usado para diferir init
                        // (`document.addEventListener('DOMContentLoaded', ...)`).
                        if t.js.is_some() {
                            let (_, pending) = dispatch_document_js_event_on_tab(
                                t,
                                "DOMContentLoaded",
                                None,
                                None,
                                now_ms,
                            );
                            for req in pending {
                                handle.dispatch(req);
                            }
                        }
                        // Fase 7.39 — disparar 'load' al window. Apps usan
                        // `window.addEventListener('load', fn)` para diferir
                        // init hasta que el DOM esté pronto. Sólo si el tab
                        // creó runtime (hay scripts en la página).
                        if t.js.is_some() {
                            let (_, pending) = dispatch_window_js_event_on_tab(t, "load", now_ms);
                            for req in pending {
                                handle.dispatch(req);
                            }
                        }
                        if t.js_summary.errors > 0 {
                            t.status =
                                format!("{} · JS: {} log/{} err",
                                    t.status, t.js_summary.logs, t.js_summary.errors);
                        } else if t.js_summary.logs > 0 {
                            t.status = format!("{} · JS: {} logs",
                                t.status, t.js_summary.logs);
                        }
                        // Registra en la history global del Profile (no
                        // confundir con TabState.history, que es el
                        // stack back/fwd de la pestaña).
                        let url_for_history = t.url.clone();
                        if let Some(handle) = profile_handle() {
                            if let Ok(mut p) = handle.lock() {
                                p.history.record(&url_for_history, &title, puriy_core::now());
                            }
                        }
                        persist_profile();
                        // <meta http-equiv="refresh"> — programa un thread
                        // que duerme N segundos y dispatcha
                        // MetaRefreshFire. El gen counter lo invalida si
                        // el usuario navegó manualmente antes de que vence.
                        if let Some(mr) = meta_refresh {
                            let resolved = mr.url.as_deref().and_then(|u| {
                                url::Url::parse(&t.url)
                                    .ok()
                                    .and_then(|base| base.join(u).ok())
                                    .map(|abs| abs.to_string())
                            });
                            t.status = match (mr.delay_secs, resolved.as_deref()) {
                                (0, Some(u)) => format!("→ refresh inmediato a {u}"),
                                (n, Some(u)) => format!("→ refresh en {n}s a {u}"),
                                (0, None) => "↻ reload inmediato".to_string(),
                                (n, None) => format!("↻ reload en {n}s"),
                            };
                            let h = handle.clone();
                            let delay = mr.delay_secs;
                            std::thread::spawn(move || {
                                if delay > 0 {
                                    std::thread::sleep(std::time::Duration::from_secs(
                                        delay as u64,
                                    ));
                                }
                                h.dispatch(Msg::MetaRefreshFire { tab, gen, url: resolved });
                            });
                        }
                    }
                }
            }
            Msg::MetaRefreshFire { tab, gen, url } => {
                // Sólo dispara si la pestaña sigue existiendo y no fue
                // pisada por otra navegación manual (gen counter).
                if let Some(idx) = m.tab_idx(tab) {
                    if m.tabs[idx].gen == gen {
                        if idx != m.active {
                            switch_active_tab(&mut m, idx);
                        }
                        let target = url.unwrap_or_else(|| m.tabs[idx].url.clone());
                        return Self::update(m, Msg::Navigate(target), handle);
                    }
                }
            }
            Msg::LoadFailed { tab, gen, err } => {
                if let Some(idx) = m.tab_idx(tab) {
                    let t = &mut m.tabs[idx];
                    if t.gen == gen {
                        t.status = format!("error: {err}");
                        t.box_tree = None;
                    }
                }
            }
            Msg::Navigate(target) => {
                // Cualquier navegación cierra el panel — el usuario quiere
                // ver la página, no la lista de bookmarks/history.
                m.panel = None;
                m.panel_filter.clear();
                m.addr_suggest.clear();
                // Same-page fragment navigation: si la URL solicitada
                // sólo difiere de la actual en el fragment, scrolleamos
                // al elemento con id matching y NO recargamos. Esto
                // matchea el comportamiento estándar de browsers para
                // `<a href="#sección">` y para typear `URL#frag` en la
                // barra estando ya en `URL`.
                let t = m.active();
                let same_doc_frag = same_doc_with_fragment(&t.url, &target);
                if let Some(frag) = same_doc_frag {
                    let y = t
                        .box_tree
                        .as_ref()
                        .and_then(|bt| bt.find_element_y(&frag));
                    let t = m.active_mut();
                    t.url = target.clone();
                    t.addr.set_text(target.clone());
                    t.history.truncate(t.cursor + 1);
                    if t.history.last() != Some(&target) {
                        t.history.push(target);
                        t.cursor = t.history.len() - 1;
                    }
                    if let Some(y) = y {
                        t.scroll_y = y.max(0.0);
                        t.status = format!("↟ #{frag}");
                    } else {
                        t.status = format!("(sin id #{frag})");
                    }
                    return m;
                }
                start_load(&mut m, target, /* push_history */ true, handle);
            }
            Msg::NavigatePost { url, body } => {
                m.panel = None;
                m.panel_filter.clear();
                start_load_post(&mut m, url, body, handle);
            }
            Msg::DownloadLink { url, filename_hint } => {
                let filename = pick_download_filename(&url, &filename_hint);
                let path = download_path(&filename);
                let status_path = path.display().to_string();
                m.active_mut().status = format!("⬇ descargando {filename}…");
                let h = handle.clone();
                let active_tab_id = m.active().id;
                let active_gen = m.active().gen;
                let url_clone = url.clone();
                std::thread::spawn(move || {
                    let result = puriy_engine::fetch::fetch_bytes(&url_clone)
                        .map_err(|e| e.to_string())
                        .and_then(|bytes| {
                            if let Some(parent) = path.parent() {
                                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
                            Ok(bytes.len())
                        });
                    h.dispatch(Msg::DownloadDone {
                        tab: active_tab_id,
                        gen: active_gen,
                        path: status_path,
                        result,
                    });
                });
            }
            Msg::DownloadDone { tab, gen, path, result } => {
                if let Some(idx) = m.tab_idx(tab) {
                    if m.tabs[idx].gen == gen {
                        m.tabs[idx].status = match result {
                            Ok(n) => format!("⬇ {path} · {n} bytes"),
                            Err(e) => format!("⬇ fallo: {e}"),
                        };
                    }
                }
            }
            Msg::NavigateNewTab(target) => {
                // `target="_blank"` debe enviar Referer del padre.
                let referer = {
                    let cur = m.active().url.clone();
                    if cur == NEW_TAB_URL || cur.is_empty() { None } else { Some(cur) }
                };
                let mut tab = TabState::new(target.clone());
                tab.gen = 1;
                spawn_load(tab.id, tab.gen, target, referer, current_viewport(), handle.clone());
                m.tabs.push(tab);
                let new_idx = m.tabs.len() - 1;
                switch_active_tab(&mut m, new_idx);
                m.panel = None;
                m.panel_filter.clear();
            }
            Msg::Scroll(dy) => {
                let now_ms = m.start.elapsed().as_millis() as u64;
                let t = m.active_mut();
                t.scroll_y = (t.scroll_y + dy).max(0.0);
                // Fase 7.39 — dispatchar 'scroll' al window para handlers
                // tipo `window.addEventListener('scroll', fn)` (header
                // sticky, infinite scroll, etc.). Sólo si hay JS runtime
                // creado para esta pestaña.
                if t.js.is_some() {
                    let (_, pending) = dispatch_window_js_event_on_tab(t, "scroll", now_ms);
                    for req in pending {
                        handle.dispatch(req);
                    }
                }
            }
            Msg::Resize(w, h) => {
                // Guardamos el viewport para que los próximos loads lo
                // sincronicen ya en la primera ejecución de scripts.
                let (vp_w, vp_h) = (w as f32, h as f32);
                PURIY_VIEWPORT.with(|c| c.set((vp_w, vp_h)));
                let now_ms = m.start.elapsed().as_millis() as u64;
                let t = m.active_mut();
                // set_viewport ANTES del dispatch para que el handler de
                // 'resize' lea `window.innerWidth`/`innerHeight` actuales.
                if let Some(rt) = t.js.as_mut() {
                    let _ = rt.set_viewport(vp_w, vp_h);
                    // Re-evalúa las media queries de ancho/alto/orientation con
                    // el viewport nuevo (dispara `change` donde flipeó).
                    sync_media_queries(rt, vp_w, vp_h, PURIY_DPR.with(|c| c.get()) as f32);
                    let (_, pending) = dispatch_window_js_event_on_tab(t, "resize", now_ms);
                    for req in pending {
                        handle.dispatch(req);
                    }
                }
            }
            Msg::ScaleFactor(scale) => {
                // Guardamos el DPR para que los próximos loads lo sincronicen
                // ya en la primera ejecución de scripts.
                PURIY_DPR.with(|c| c.set(scale));
                let now_ms = m.start.elapsed().as_millis() as u64;
                let t = m.active_mut();
                // set_device_pixel_ratio ANTES del dispatch para que el
                // handler de 'resize' lea `window.devicePixelRatio` actual
                // (los browsers disparan 'resize' al cambiar el DPI).
                if let Some(rt) = t.js.as_mut() {
                    let _ = rt.set_device_pixel_ratio(scale);
                    // Re-evalúa las media queries de resolution con el DPR nuevo.
                    let (vp_w, vp_h) = PURIY_VIEWPORT.with(|c| c.get());
                    sync_media_queries(rt, vp_w, vp_h, scale as f32);
                    let (_, pending) = dispatch_window_js_event_on_tab(t, "resize", now_ms);
                    for req in pending {
                        handle.dispatch(req);
                    }
                }
            }
            Msg::FocusAddr => {
                m.active_mut().addr_focused = true;
                // Al enfocar, sembrá sugerencias contra el texto actual (vacío
                // ⇒ sin dropdown hasta que el usuario teclee).
                let q = m.active().addr.text();
                m.addr_suggest = compute_addr_suggestions(&q);
            }
            Msg::AddrKey(e) => {
                if matches!(&e.key, Key::Named(NamedKey::Enter)) {
                    let raw = m.active().addr.text().trim().to_string();
                    if !raw.is_empty() {
                        m.addr_suggest.clear();
                        // Repotenciado: "buscar-o-navegar". Si parece URL/dominio
                        // navega; si no, lo manda al buscador.
                        let target = normalize_omnibox_input(&raw);
                        return Self::update(m, Msg::Navigate(target), handle);
                    }
                } else if matches!(&e.key, Key::Named(NamedKey::Escape)) {
                    let t = m.active_mut();
                    t.addr_focused = false;
                    t.addr.set_text(t.url.clone());
                    m.addr_suggest.clear();
                } else {
                    m.active_mut().addr.apply_key(&e);
                    // Recomputá las sugerencias de autocompletar (historial +
                    // marcadores) contra el texto vigente.
                    let q = m.active().addr.text();
                    m.addr_suggest = compute_addr_suggestions(&q);
                }
            }
            Msg::Back => {
                let t = m.active_mut();
                if t.can_back() {
                    t.cursor -= 1;
                    let url = t.history[t.cursor].clone();
                    start_load(&mut m, url, /* push_history */ false, handle);
                }
            }
            Msg::Forward => {
                let t = m.active_mut();
                if t.can_fwd() {
                    t.cursor += 1;
                    let url = t.history[t.cursor].clone();
                    start_load(&mut m, url, /* push_history */ false, handle);
                }
            }
            Msg::NewTab => {
                let mut t = TabState::new(NEW_TAB_URL.into());
                t.status = "nueva pestaña".into();
                t.box_tree = None;
                // La pestaña nueva nace en el space activo (en horizontal hay
                // un solo space visible; en vertical, bajo el diente activo).
                t.space = m.active_space;
                m.tabs.push(t);
                let new_idx = m.tabs.len() - 1;
                switch_active_tab(&mut m, new_idx);
                m.active_mut().addr_focused = true;
            }
            Msg::CloseTab(idx) => {
                let closing_active = idx == m.active;
                let closed_space = m.tabs.get(idx).map(|t| t.space).unwrap_or(m.active_space);
                if idx < m.tabs.len() {
                    // Corta los EventSource de la pestaña antes de tirarla.
                    m.tabs[idx].cancel_all_eventsources();
                    m.tabs.remove(idx);
                }
                if m.tabs.is_empty() {
                    // No quedan pestañas: sembrá una en el space que se vació.
                    let mut t = TabState::new(NEW_TAB_URL.into());
                    t.space = m.active_space;
                    m.tabs.push(t);
                    m.active = 0;
                } else if m.active >= m.tabs.len() {
                    // El active quedó out-of-bounds tras el remove — apuntá al
                    // último (sin switch: la tab vieja ya no existe).
                    m.active = m.tabs.len() - 1;
                    if let Some(rt) = m.tabs[m.active].js.as_mut() {
                        let _ = rt.set_visibility(false);
                    }
                } else if closing_active {
                    // Cerramos la activa pero el índice sigue válido (apunta a
                    // lo que ocupó su lugar). Si esa pestaña cayó en otro space,
                    // preferí una del space que estábamos viendo para no saltar
                    // de contexto.
                    if m.tabs[m.active].space != closed_space {
                        if let Some(&sib) = m.tabs_in_space(closed_space).first() {
                            m.active = sib;
                        } else {
                            // El space quedó sin pestañas: seguí el space de la
                            // pestaña que ocupó el hueco.
                            m.active_space = m.tabs[m.active].space;
                        }
                    }
                    if let Some(rt) = m.tabs[m.active].js.as_mut() {
                        let _ = rt.set_visibility(false);
                    }
                }
            }
            Msg::SelectTab(idx) => {
                if idx < m.tabs.len() && idx != m.active {
                    switch_active_tab(&mut m, idx);
                }
            }
            Msg::NextTab => {
                // Cicla dentro del space activo (con wrap). Si el space tiene
                // una sola pestaña, no-op.
                let sibs = m.active_space_tabs();
                if sibs.len() > 1 {
                    let here = sibs.iter().position(|&i| i == m.active).unwrap_or(0);
                    let next = sibs[(here + 1) % sibs.len()];
                    switch_active_tab(&mut m, next);
                }
            }
            Msg::PrevTab => {
                let sibs = m.active_space_tabs();
                if sibs.len() > 1 {
                    let here = sibs.iter().position(|&i| i == m.active).unwrap_or(0);
                    let prev = sibs[(here + sibs.len() - 1) % sibs.len()];
                    switch_active_tab(&mut m, prev);
                }
            }
            Msg::Bookmark => {
                let t = m.active();
                let url = t.url.clone();
                let title = if t.title.is_empty() { t.url.clone() } else { t.title.clone() };
                if let Some(handle) = profile_handle() {
                    if let Ok(mut p) = handle.lock() {
                        let already = p
                            .bookmarks
                            .items()
                            .iter()
                            .any(|b| b.url == url);
                        if !already {
                            p.bookmarks.add(&url, &title, None, puriy_core::now());
                            m.active_mut().status = format!("⭐ guardado · {} bookmarks", p.bookmarks.len());
                        } else {
                            m.active_mut().status = "⭐ ya estaba guardado".into();
                        }
                    }
                }
                persist_profile();
            }
            Msg::ZoomIn => {
                let new_zoom = (m.zoom * ZOOM_STEP).min(ZOOM_MAX);
                m.zoom = new_zoom;
                m.active_mut().status = format!("zoom: {}%", (new_zoom * 100.0).round() as i32);
            }
            Msg::ZoomOut => {
                let new_zoom = (m.zoom / ZOOM_STEP).max(ZOOM_MIN);
                m.zoom = new_zoom;
                m.active_mut().status = format!("zoom: {}%", (new_zoom * 100.0).round() as i32);
            }
            Msg::ZoomReset => {
                m.zoom = 1.0;
                m.active_mut().status = "zoom: 100%".into();
            }
            Msg::FindOpen => {
                m.find_open();
            }
            Msg::FindClose => {
                m.find_close();
            }
            Msg::FindKey(e) => {
                let before = m.find_input.text();
                m.find_input.apply_key(&e);
                let after = m.find_input.text();
                if before != after {
                    // Query cambió → cualquier "match actual" previo
                    // queda inválido. Esperamos el primer Enter para
                    // arrancar la navegación.
                    m.find_current = 0;
                }
            }
            Msg::FindNext => {
                m.find_step(true);
            }
            Msg::FindPrev => {
                m.find_step(false);
            }
            Msg::FindToggleCase => {
                m.find_case_sensitive = !m.find_case_sensitive;
                // El conjunto de matches cambió → reseteamos la nav; el
                // próximo Enter arranca desde el primer match nuevo.
                m.find_current = 0;
            }
            Msg::FindToggleWord => {
                m.find_whole_word = !m.find_whole_word;
                m.find_current = 0;
            }
            Msg::ToggleBookmarks => {
                m.panel = match m.panel {
                    Some(PanelKind::Bookmarks) => None,
                    _ => Some(PanelKind::Bookmarks),
                };
                m.panel_filter.clear();
            }
            Msg::ToggleHistory => {
                m.panel = match m.panel {
                    Some(PanelKind::History) => None,
                    _ => Some(PanelKind::History),
                };
                m.panel_filter.clear();
            }
            Msg::ClosePanel => {
                m.panel = None;
                m.panel_filter.clear();
            }
            Msg::RemoveBookmark(id) => {
                if let Some(handle) = profile_handle() {
                    if let Ok(mut p) = handle.lock() {
                        if p.bookmarks.remove(id) {
                            m.active_mut().status =
                                format!("⭐ borrado · {} bookmarks", p.bookmarks.len());
                        }
                    }
                }
                persist_profile();
            }
            Msg::ToggleDetails(idx) => {
                let t = m.active_mut();
                if let Some(slot) = t.details_open.get_mut(idx) {
                    *slot = !*slot;
                }
            }
            Msg::HoverTween { node_id, entering, duration_ms } => {
                let now_ms = m.start.elapsed().as_millis() as u64;
                let t = m.active_mut();
                // Captura el progreso lineal al instante del toggle y reancla
                // el reloj con la nueva dirección — así un enter→leave rápido
                // revierte desde donde iba, sin saltar a 0/1.
                let prev = t.hover_tweens.get(&node_id).copied();
                let progress_at_toggle =
                    prev.map(|tw| tw.sample_linear(now_ms)).unwrap_or(0.0);
                t.hover_tweens.insert(
                    node_id,
                    HoverTween { hovered: entering, progress_at_toggle, toggle_ms: now_ms, duration_ms },
                );
            }
            Msg::JsTick => {
                let now_ms = m.start.elapsed().as_millis() as u64;
                for req in tick_js_runtimes(&mut m, now_ms) {
                    handle.dispatch(req);
                }
            }
            req @ Msg::FetchRequest { .. } => {
                spawn_fetch(req, handle.clone());
            }
            Msg::SetSystemClipboard(text) => {
                // `navigator.clipboard.writeText`/`write` → portapapeles real
                // (Fase 7.176). Degrada a no-op si no hay backend (headless).
                m.clipboard.set(&text);
            }
            Msg::EsOpen { tab, gen, es_id, url } => {
                // Abre el stream SSE (Fase 7.182). Sólo si la pestaña sigue viva
                // y en el mismo gen (no se navegó mientras tanto).
                if let Some(idx) = m.tab_idx(tab) {
                    if m.tabs[idx].gen == gen {
                        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                        m.tabs[idx].es_cancel.insert(es_id, cancel.clone());
                        spawn_eventsource(tab, gen, es_id, url, cancel, handle.clone());
                    }
                }
            }
            Msg::EsClose { tab, es_id } => {
                if let Some(idx) = m.tab_idx(tab) {
                    if let Some(flag) = m.tabs[idx].es_cancel.remove(&es_id) {
                        flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
            Msg::EsDispatch { tab, gen, es_id, kind, event_type, data, last_id } => {
                if let Some(idx) = m.tab_idx(tab) {
                    if m.tabs[idx].gen == gen {
                        let now_ms = m.start.elapsed().as_millis() as u64;
                        let t = &mut m.tabs[idx];
                        if let Some(rt) = t.js.as_mut() {
                            let _ = rt.set_now_ms(now_ms);
                            rt.set_fuel(puriy_js::DEFAULT_FUEL);
                            let _ = rt.es_dispatch(es_id, &kind, &event_type, &data, &last_id);
                            // Un handler SSE puede haber tocado el DOM.
                            for req in apply_dom_mutations(t) {
                                handle.dispatch(req);
                            }
                        }
                    }
                }
            }
            Msg::FetchComplete { tab, gen, fetch_id, result } => {
                let tab_idx = m.tabs.iter().position(|t| t.id == tab && t.gen == gen);
                if let Some(idx) = tab_idx {
                    if let Some(rt) = m.tabs[idx].js.as_mut() {
                        let now_ms = m.start.elapsed().as_millis() as u64;
                        let _ = rt.set_now_ms(now_ms);
                        rt.set_fuel(puriy_js::DEFAULT_FUEL);
                        let prev_stdout = rt.stdout().len();
                        let prev_stderr = rt.stderr().len();
                        match result {
                            Ok(resp) => {
                                let body_str = String::from_utf8_lossy(&resp.body).into_owned();
                                let _ = rt.resolve_fetch(
                                    fetch_id,
                                    resp.status,
                                    &resp.status_text,
                                    &body_str,
                                    &resp.headers,
                                );
                            }
                            Err(err) => {
                                let _ = rt.reject_fetch(fetch_id, &err);
                            }
                        }
                        let new_stdout = rt.stdout();
                        let new_stderr = rt.stderr();
                        m.tabs[idx].js_summary.logs +=
                            new_stdout[prev_stdout..].matches('\n').count();
                        m.tabs[idx].js_summary.errors +=
                            new_stderr[prev_stderr..].matches('\n').count();
                        // Las mutaciones resultantes (fetch encadenado, write
                        // al portapapeles) se despachan al loop: `FetchRequest`
                        // re-entra a su arm (que spawnea el worker) y
                        // `SetSystemClipboard` al suyo.
                        for next in apply_dom_mutations(&mut m.tabs[idx]) {
                            handle.dispatch(next);
                        }
                    }
                }
            }
            Msg::JsDispatchEvent { element_id, event_type, fallback } => {
                let now_ms = m.start.elapsed().as_millis() as u64;
                let (result, pending) = dispatch_js_event(&mut m, &element_id, &event_type, now_ms);
                for req in pending {
                    handle.dispatch(req);
                }
                // Si el handler JS no llamó `event.preventDefault()` y
                // hay un fallback (típicamente Navigate del `<a href>`),
                // lo reenviamos al event loop para que el chrome lo
                // procese normalmente. `dispatch` despacha el msg en el
                // próximo iteración de update.
                if !result.default_prevented {
                    if let Some(fb) = fallback {
                        handle.dispatch(*fb);
                    }
                }
            }
            Msg::PanelFilterKey(e) => {
                m.panel_filter.apply_key(&e);
            }
            Msg::ViewSource => {
                m.panel = match m.panel {
                    Some(PanelKind::Source) => None,
                    _ => Some(PanelKind::Source),
                };
                m.panel_filter.clear();
            }
            Msg::HoverLink(url) => {
                m.hover_link = url;
            }
            Msg::SelectToggle(idx) => {
                let t = m.active_mut();
                if let Some(s) = t.selects.get_mut(idx) {
                    s.open = !s.open;
                }
            }
            Msg::SelectPick(idx, opt) => {
                let t = m.active_mut();
                if let Some(s) = t.selects.get_mut(idx) {
                    s.selected = opt;
                    s.open = false;
                }
                // Fase 7.7: despachar `change` JS si el <select> tiene id.
                let eid = m
                    .active()
                    .selects_element_ids
                    .get(idx)
                    .cloned()
                    .flatten();
                if let Some(eid) = eid {
                    let now_ms = m.start.elapsed().as_millis() as u64;
                    // Fase 7.9 — pasar el value del option recién
                    // seleccionado en el EventInit. Así el handler de
                    // `change` lee `event.target.value` y obtiene el
                    // value del option, no el label.
                    let value = select_value_at(m.active(), idx, opt);
                    let mut init = puriy_js::EventInit::default();
                    init.value = value;
                    let (_, pending) = dispatch_js_event_with_init(
                        &mut m,
                        &eid,
                        "change",
                        now_ms,
                        Some(init),
                    );
                    for req in pending { handle.dispatch(req); }
                }
            }
            Msg::ToggleCheckbox(idx) => {
                let t = m.active_mut();
                if let Some(c) = t.input_checks.get_mut(idx) {
                    *c = !*c;
                }
                // Fase 7.187 — refleja el nuevo estado en el atributo `checked`
                // del box y recascadea para que `:checked`/`:checked + label`
                // se actualicen al togglear en vivo.
                let checks = t.input_checks.clone();
                if let Some(bt) = t.box_tree.as_mut() {
                    bt.sync_checked_from(&checks);
                    bt.restyle();
                }
            }
            Msg::SelectRadio(idx) => {
                // Encontrá el `name` de este radio + form_idx; los radios
                // del mismo grupo se desmarcan, éste queda marcado.
                let tree_opt = m.active().box_tree.clone();
                let Some(tree) = tree_opt else { return m };
                let mut my_name: Option<String> = None;
                let mut my_form: Option<usize> = None;
                let mut i = 0usize;
                tree.walk(|b| {
                    if b.input_kind.is_some() {
                        if i == idx {
                            my_name = b.input_name.clone();
                            my_form = b.form_idx;
                        }
                        i += 1;
                    }
                });
                let mut counter = 0usize;
                let t = m.active_mut();
                tree.walk(|b| {
                    if b.input_kind == Some(puriy_engine::InputKind::Radio)
                        && b.input_name == my_name
                        && b.form_idx == my_form
                    {
                        if let Some(slot) = t.input_checks.get_mut(counter) {
                            *slot = counter == idx;
                        }
                    }
                    if b.input_kind.is_some() {
                        counter += 1;
                    }
                });
                // Fase 7.187 — espeja el estado del grupo de radios al atributo
                // `checked` de los boxes y recascadea (`:checked` en vivo).
                let checks = t.input_checks.clone();
                if let Some(bt) = t.box_tree.as_mut() {
                    bt.sync_checked_from(&checks);
                    bt.restyle();
                }
            }
            Msg::SubmitForm(idx) => {
                // Tratamos como si el input idx estuviera focado.
                m.active_mut().focused_input = Some(idx);
                if let Some(msg) = build_form_submit_url(&m) {
                    return Self::update(m, msg, handle);
                }
            }
            Msg::FocusInput(idx) => {
                // Fase 7.7: despachar blur al input previo (si tenía id)
                // y focus al nuevo (si tiene id).
                let prev = m.active().focused_input;
                let prev_eid = prev.and_then(|i| {
                    m.active()
                        .inputs_element_ids
                        .get(i)
                        .cloned()
                        .flatten()
                });
                let new_eid = if idx == usize::MAX {
                    None
                } else {
                    m.active()
                        .inputs_element_ids
                        .get(idx)
                        .cloned()
                        .flatten()
                };
                let t = m.active_mut();
                if idx == usize::MAX {
                    // sentinel = blur
                    t.focused_input = None;
                } else if idx < t.inputs.len() {
                    t.focused_input = Some(idx);
                    // Blur address bar para que las teclas no compitan.
                    t.addr_focused = false;
                }
                let now_ms = m.start.elapsed().as_millis() as u64;
                if let Some(eid) = prev_eid {
                    let (_, p) = dispatch_js_event(&mut m, &eid, "blur", now_ms);
                    for req in p { handle.dispatch(req); }
                }
                if let Some(eid) = new_eid {
                    let (_, p) = dispatch_js_event(&mut m, &eid, "focus", now_ms);
                    for req in p { handle.dispatch(req); }
                }
            }
            Msg::FocusNext => {
                let t = m.active_mut();
                if !t.inputs.is_empty() {
                    let n = t.inputs.len();
                    let next = match t.focused_input {
                        Some(i) => (i + 1) % n,
                        None => 0,
                    };
                    t.focused_input = Some(next);
                }
            }
            Msg::FocusPrev => {
                let t = m.active_mut();
                if !t.inputs.is_empty() {
                    let n = t.inputs.len();
                    let prev = match t.focused_input {
                        Some(0) | None => n - 1,
                        Some(i) => i - 1,
                    };
                    t.focused_input = Some(prev);
                }
            }
            Msg::InputKey(e) => {
                if matches!(&e.key, Key::Named(NamedKey::Enter)) {
                    // Submit (GET o POST según el form method).
                    if let Some(submit_msg) = build_form_submit_url(&m) {
                        return Self::update(m, submit_msg, handle);
                    } else {
                        m.active_mut().status =
                            "↵ submit: el input no está dentro de un <form action> conocido".into();
                    }
                } else {
                    // Fase 7.7: despachar `keydown` al elemento focado si
                    // tiene `id=`. Si el handler hace `preventDefault()`,
                    // la tecla NO se aplica al input — el JS toma el control.
                    let focused_idx = m.active().focused_input;
                    let eid = focused_idx.and_then(|i| {
                        m.active()
                            .inputs_element_ids
                            .get(i)
                            .cloned()
                            .flatten()
                    });
                    let prevented = if let Some(eid) = eid {
                        let now_ms = m.start.elapsed().as_millis() as u64;
                        // Fase 7.9 — enriquecer el event object con key/
                        // code/modifiers + value actual del input. Así un
                        // handler puede leer event.key === 'Enter' o
                        // event.target.value antes de aplicar el keydown.
                        let focused_idx = m.active().focused_input;
                        let mut init = key_event_to_init(&e);
                        if let Some(idx) = focused_idx {
                            if let Some(input) = m.active().inputs.get(idx) {
                                init.value = Some(input.text());
                            }
                        }
                        {
                            let (r, p) = dispatch_js_event_with_init(
                                &mut m,
                                &eid,
                                "keydown",
                                now_ms,
                                Some(init),
                            );
                            for req in p { handle.dispatch(req); }
                            r.default_prevented
                        }
                    } else {
                        false
                    };
                    if !prevented {
                        let mut new_value: Option<String> = None;
                        let mut input_eid: Option<String> = None;
                        let t = m.active_mut();
                        if let Some(idx) = t.focused_input {
                            if let Some(input) = t.inputs.get_mut(idx) {
                                input.apply_key(&e);
                                new_value = Some(input.text());
                                input_eid = t
                                    .inputs_element_ids
                                    .get(idx)
                                    .cloned()
                                    .flatten();
                            }
                        }
                        // Fase 7.10 — `input` event DESPUÉS de aplicar la
                        // tecla (a diferencia de `keydown` que se dispara
                        // ANTES). Handlers de autocomplete/search-as-you-
                        // type leen `event.target.value` con el value
                        // recién actualizado.
                        if let (Some(eid), Some(v)) = (input_eid, new_value) {
                            let now_ms = m.start.elapsed().as_millis() as u64;
                            let mut init = puriy_js::EventInit::default();
                            init.value = Some(v);
                            let (_, p) = dispatch_js_event_with_init(
                                &mut m,
                                &eid,
                                "input",
                                now_ms,
                                Some(init),
                            );
                            for req in p { handle.dispatch(req); }
                        }
                    }
                }
            }
            Msg::MenuOpen(idx) => {
                m.menu_open = idx;
                m.edit_menu = None;
                m.menu_active = usize::MAX;
                if idx.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuCommand(cmd) => {
                return handle_menu_command(m, cmd, handle);
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    m.menu_active = menubar_nav(&menu, mi, m.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    if let Some(cmd) = menubar_command_at(&menu, mi, m.menu_active) {
                        return handle_menu_command(m, cmd, handle);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::EditNav(dir) => {
                let flags = match m.focused_text_input() {
                    Some((input, masked)) => EditFlags::from_editor(input.editor(), masked),
                    None => EditFlags::default(),
                };
                m.edit_active = editmenu::edit_menu_step(flags, m.edit_active, dir);
            }
            Msg::EditActivate => {
                let flags = match m.focused_text_input() {
                    Some((input, masked)) => EditFlags::from_editor(input.editor(), masked),
                    None => EditFlags::default(),
                };
                if let Some(a) = editmenu::edit_menu_action_at(flags, m.edit_active) {
                    apply_edit_menu_action(&mut m, a);
                }
            }
            Msg::CloseMenus => {
                m.menu_open = None;
                m.edit_menu = None;
                m.menu_active = usize::MAX;
                m.edit_active = usize::MAX;
            }
            Msg::EditMenuOpen(x, y) => {
                m.edit_menu = Some((x, y));
                m.menu_open = None;
                m.edit_active = usize::MAX;
                m.edit_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                animate(handle, motion::FAST, || Msg::MenuTick);
            }
            Msg::EditMenuAction(action) => {
                apply_edit_menu_action(&mut m, action);
            }
            Msg::NewSpace => {
                let n = m.spaces.len();
                // Glifo rotativo para el diente nuevo (ciclo corto y legible).
                const GLYPHS: [&str; 8] = ["◆", "●", "▲", "■", "★", "✦", "◈", "❖"];
                m.spaces.push(Space::new(
                    format!("Space {}", n + 1),
                    GLYPHS[n % GLYPHS.len()],
                ));
                m.active_space = n;
                // Un space nace con una pestaña vacía adentro.
                let mut t = TabState::new(NEW_TAB_URL.into());
                t.status = "nuevo space".into();
                t.box_tree = None;
                t.space = n;
                m.tabs.push(t);
                let new_idx = m.tabs.len() - 1;
                switch_active_tab(&mut m, new_idx);
                m.active_mut().addr_focused = true;
                persist_ui_prefs(&m);
            }
            Msg::SelectSpace(idx) => {
                if idx < m.spaces.len() && idx != m.active_space {
                    m.active_space = idx;
                    // Enfocá la última pestaña de ese space; si no tiene ninguna
                    // (caso raro tras mover pestañas), creá una vacía.
                    match m.tabs_in_space(idx).last().copied() {
                        Some(tab_idx) => switch_active_tab(&mut m, tab_idx),
                        None => {
                            let mut t = TabState::new(NEW_TAB_URL.into());
                            t.space = idx;
                            m.tabs.push(t);
                            let new_idx = m.tabs.len() - 1;
                            switch_active_tab(&mut m, new_idx);
                        }
                    }
                }
            }
            Msg::MoveTabToSpace { tab_idx, dest } => {
                if tab_idx < m.tabs.len() && dest < m.spaces.len() {
                    m.tabs[tab_idx].space = dest;
                }
            }
            Msg::OpenSettings => {
                m.settings_open = true;
                m.settings = AllichayState::new();
            }
            Msg::CloseSettings => {
                m.settings_open = false;
            }
            Msg::Settings(amsg) => {
                apply_settings_msg(&mut m, amsg);
            }
            Msg::AddrSuggestPick(url) => {
                m.addr_suggest.clear();
                m.active_mut().addr_focused = false;
                return Self::update(m, Msg::Navigate(url), handle);
            }
        }
        m
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        // Prioridad: panel de configuración > menú contextual de edición >
        // dropdown del menú principal > overlay del `<select>` abierto.
        if model.settings_open {
            return Some(settings_overlay_view(model));
        }
        if let Some((x, y)) = model.edit_menu {
            let flags = match model.focused_text_input() {
                Some((input, masked)) => EditFlags::from_editor(input.editor(), masked),
                None => EditFlags::default(),
            };
            let (w, h) = Self::initial_size();
            let mut spec = editmenu::edit_context_menu(
                (x, y),
                (w as f32, h as f32),
                menu_theme(),
                flags,
                Msg::EditMenuAction,
                Msg::CloseMenus,
            );
            spec.active = model.edit_active;
            return Some(context_menu_view_ex(
                spec,
                ContextMenuExtras { appear: model.edit_anim.value(), ..Default::default() },
            ));
        }
        let menu = app_menu(model);
        if let Some(ov) = menubar_overlay_animated(
            &menubar_spec(&menu, model),
            model.menu_active,
            model.menu_anim.value(),
        ) {
            return Some(ov);
        }

        // Si algún `<select>` está abierto, mostramos su lista como un
        // overlay centrado. Sin layout positioning real (no sabemos
        // dónde quedó el header del select en pantalla), el centrado
        // es la opción menos sorprendente. Click en una opción cierra;
        // backdrop transparente cierra también.
        let t = model.active();
        let (sel_idx, sel_state) = t
            .selects
            .iter()
            .enumerate()
            .find(|(_, s)| s.open)?;
        // Busca el SelectInfo correspondiente en el box tree por DFS idx.
        let tree = t.box_tree.as_ref()?;
        let mut info: Option<puriy_engine::SelectInfo> = None;
        let mut counter = 0usize;
        tree.walk(|b| {
            if let Some(s) = &b.select {
                if counter == sel_idx {
                    info = Some(s.clone());
                }
                counter += 1;
            }
        });
        let info = info?;
        Some(select_overlay_view(sel_idx, sel_state.selected, info))
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        // Header renovado (theme-driven): nav + indicador de seguridad + URL
        // repotenciada + autocompletar. Compartido por ambas orientaciones.
        let header = nav_header_bar(model);
        // Predicado de búsqueda activo (query + toggles case/whole-word).
        // Si la find bar está cerrada, un matcher vacío → sin highlight.
        let matcher = if model.find_active {
            model.find_matcher()
        } else {
            Matcher::new("", MatchOpts::default())
        };
        // Pre-cuenta los matches del documento para mostrarlos en la
        // find bar. Matcher vacío (bar cerrada / query vacía) → count 0.
        let find_count = count_matches(model.active().box_tree.as_ref(), &matcher);
        let body = match model.panel {
            Some(kind) => panel_view(
                kind,
                &model.panel_filter,
                model.active().source.as_deref(),
                model.zoom,
            ),
            None => {
                // Elapsed para el runtime de animaciones CSS: now − ancla de
                // la carga. El tick periódico (JsTick, ~30fps) re-renderiza,
                // así que leer el reloj acá avanza la animación cada frame.
                let now_ms = model.start.elapsed().as_millis() as u64;
                let anim_elapsed_ms = now_ms.saturating_sub(model.active().anim_start_ms);
                viewport(
                    model.active(),
                    model.zoom,
                    &matcher,
                    model.find_current,
                    anim_elapsed_ms,
                    now_ms,
                )
            }
        };

        let find = if model.find_active {
            Some(find_bar(
                &model.find_input,
                find_count,
                model.find_current,
                model.find_case_sensitive,
                model.find_whole_word,
            ))
        } else {
            None
        };

        // Barra de menú principal — PRIMER hijo del column raíz, full width en
        // ambas orientaciones.
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let children: Vec<View<Msg>> = match model.orientation {
            TabOrientation::Horizontal => {
                // Un nivel: barra de pestañas del space activo arriba.
                let mut c: Vec<View<Msg>> = vec![menubar, tabs_bar(model), header];
                if let Some(f) = find {
                    c.push(f);
                }
                c.push(body);
                c
            }
            TabOrientation::Vertical => {
                // Sidebar de dientes a la izquierda; header + body a la derecha.
                let mut main: Vec<View<Msg>> = vec![header];
                if let Some(f) = find {
                    main.push(f);
                }
                main.push(body);
                let main_col = View::new(Style {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
                    ..Default::default()
                })
                .children(main);
                let row = View::new(Style {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Row,
                    size: Size { width: percent(1.0_f32), height: percent(0.0_f32) },
                    ..Default::default()
                })
                .children(vec![sidebar_view(model), main_col]);
                vec![menubar, row]
            }
        };

        // Right-click en la raíz (origen 0,0 → las coords locales que
        // llegan al handler ya son de ventana) abre el menú contextual de
        // edición sobre el campo de texto focuseado.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(model.theme.bg_app)
        .on_right_click_at(|x, y, _w, _h| Some(Msg::EditMenuOpen(x, y)))
        .children(children)
    }
}
