#[allow(unused_imports)] use super::*;
#[allow(unused_imports)] use super::super::*;
#[allow(unused_imports)] use llimphi_raster::kurbo::{Cap, Join};
#[allow(unused_imports)] use llimphi_raster::peniko::{Brush, Extend};



    #[test]
    fn toggle_whole_word_filtra_substrings() {
        let mut m = model_con_doc("<p>cat</p><p>category</p>");
        m.find_open();
        m.find_input.set_text("cat");
        m.find_whole_word = true;
        let total = count_matches(m.active().box_tree.as_ref(), &m.find_matcher());
        assert_eq!(total, 1, "whole-word excluye 'category'");
    }

    #[test]
    fn skip_count_details_avanza_por_cada_details_anidado() {
        let tree = parse(
            "<details><summary>A</summary><details><summary>B</summary><p>x</p></details></details>\
             <details><summary>C</summary></details>",
        );
        // Pre-cuenta total via walk (mismo orden que el Loaded llena).
        let mut total = 0_usize;
        tree.walk(|b| {
            if b.tag.as_deref() == Some("details") {
                total += 1;
            }
        });
        assert!(total >= 3, "esperaba >= 3 <details>, conseguí {total}");

        let mut counter = 0_usize;
        skip_count_details(&tree.root, &mut counter);
        assert_eq!(counter, total, "skip_count_details debe contar todos los <details>");
    }

    #[test]
    fn skip_count_details_no_cuenta_otros_tags() {
        let tree = parse("<p>foo</p><h1>bar</h1><div><span>baz</span></div>");
        let mut counter = 0_usize;
        skip_count_details(&tree.root, &mut counter);
        assert_eq!(counter, 0);
    }

    #[test]
    fn extract_body_text_concatena_hojas() {
        let tree = parse("<body><h1>Hola</h1><p>mundo cruel</p></body>");
        let text = extract_body_text(&tree);
        assert!(text.contains("Hola"));
        assert!(text.contains("mundo cruel"));
    }

    #[test]
    fn run_scripts_actualiza_summary_logs() {
        let mut t = TabState::new("about:test".into());
        t.title = "T".into();
        t.url = "about:test".into();
        t.box_tree = Some(parse("<p>x</p>"));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some("console.log('a'); console.log('b')".into()),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        assert_eq!(t.js_summary.logs, 2, "esperaba 2 logs");
        assert_eq!(t.js_summary.errors, 0);
        // El runtime debe haberse instanciado.
        assert!(t.js.is_some());
    }

    #[test]
    fn run_scripts_captura_error_thrown() {
        let mut t = TabState::new("about:test".into());
        t.box_tree = Some(parse("<p>x</p>"));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some("console.log('ok'); throw new Error('boom')".into()),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        // 1 log de console + 1 error del throw.
        assert_eq!(t.js_summary.logs, 1);
        assert_eq!(t.js_summary.errors, 1);
    }

    #[test]
    fn run_scripts_saltea_modules_y_src_externo() {
        let mut t = TabState::new("about:test".into());
        t.box_tree = Some(parse("<p>x</p>"));
        let scripts = vec![
            puriy_engine::ScriptInfo {
                src: Some("/main.js".into()),
                inline: None,
                type_attr: None,
                is_module: false,
                defer: false,
                async_: false,
            },
            puriy_engine::ScriptInfo {
                src: None,
                inline: Some("console.log('module')".into()),
                type_attr: Some("module".into()),
                is_module: true,
                defer: false,
                async_: false,
            },
        ];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        // Ninguno de los dos ejecutable → no se instancia runtime.
        assert!(t.js.is_none());
        assert_eq!(t.js_summary.logs, 0);
        assert_eq!(t.js_summary.errors, 0);
    }

    #[test]
    fn run_scripts_documento_inyecta_title_y_url() {
        let mut t = TabState::new("https://example.com/x".into());
        t.title = "Hola mundo".into();
        t.box_tree = Some(parse("<p>cuerpo</p>"));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some(
                "console.log(document.title); console.log(document.URL)".into(),
            ),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        let rt = t.js.as_ref().expect("rt creado");
        let out = rt.stdout();
        assert!(out.contains("Hola mundo"), "stdout: {out:?}");
        assert!(out.contains("https://example.com/x"), "stdout: {out:?}");
    }

    #[test]
    fn run_scripts_skip_application_json_pero_no_text_javascript() {
        let mut t = TabState::new("about:test".into());
        t.box_tree = Some(parse("<p>x</p>"));
        let scripts = vec![
            puriy_engine::ScriptInfo {
                src: None,
                inline: Some("{\"k\":1}".into()),
                type_attr: Some("application/json".into()),
                is_module: false,
                defer: false,
                async_: false,
            },
            puriy_engine::ScriptInfo {
                src: None,
                inline: Some("console.log('ejecuto')".into()),
                type_attr: Some("text/javascript".into()),
                is_module: false,
                defer: false,
                async_: false,
            },
        ];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        assert_eq!(t.js_summary.logs, 1);
    }

    #[test]
    fn tick_dispara_settimeout_pendiente() {
        let mut m = model_con_script("setTimeout(function(){ console.log('tic') }, 100)");
        assert!(m.tabs[0].js.is_some());
        let logs_pre = m.tabs[0].js_summary.logs;
        tick_js_runtimes(&mut m, 50);
        assert_eq!(m.tabs[0].js_summary.logs, logs_pre);
        tick_js_runtimes(&mut m, 100);
        assert_eq!(m.tabs[0].js_summary.logs, logs_pre + 1);
    }

    #[test]
    fn tick_no_panic_en_pestana_sin_js() {
        let mut t = TabState::new("about:test".into());
        t.box_tree = Some(parse("<p>x</p>"));
        let mut m = Model {
            tabs: vec![t],
            active: 0,
            spaces: vec![Space::new("Principal", "◆")],
            active_space: 0,
            orientation: TabOrientation::Horizontal,
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
        };
        tick_js_runtimes(&mut m, 1234);
        assert!(m.tabs[0].js.is_none());
        assert_eq!(m.tabs[0].js_summary.logs, 0);
    }

    #[test]
    fn tick_acumula_errores_en_summary() {
        let mut m = model_con_script(
            "setTimeout(function(){ throw new Error('boom') }, 10)",
        );
        let errs_pre = m.tabs[0].js_summary.errors;
        tick_js_runtimes(&mut m, 50);
        assert!(
            m.tabs[0].js_summary.errors > errs_pre,
            "esperaba al menos 1 error nuevo en summary"
        );
    }

    #[test]
    fn tick_continua_disparando_interval() {
        let mut m = model_con_script(
            "setInterval(function(){ console.log('p') }, 20)",
        );
        let logs0 = m.tabs[0].js_summary.logs;
        tick_js_runtimes(&mut m, 20);
        tick_js_runtimes(&mut m, 40);
        tick_js_runtimes(&mut m, 60);
        assert_eq!(m.tabs[0].js_summary.logs, logs0 + 3);
    }

    #[test]
    fn collect_element_snapshots_indexa_solo_los_con_id() {
        let tree = parse(
            r#"<div><h1 id="hero">Título</h1><p>sin id</p><button id="b">x</button></div>"#,
        );
        let snaps = collect_element_snapshots(&tree);
        let ids: Vec<&str> = snaps.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"hero"), "snaps: {snaps:?}");
        assert!(ids.contains(&"b"), "snaps: {snaps:?}");
        assert_eq!(ids.len(), 2, "snaps: {snaps:?}");
    }

    #[test]
    fn collect_element_snapshots_text_content_concatena_subarbol() {
        let tree = parse(r#"<div id="x"><span>uno</span> <b>dos</b></div>"#);
        let snaps = collect_element_snapshots(&tree);
        let x = snaps.iter().find(|s| s.id == "x").expect("id=x");
        assert!(x.text_content.contains("uno"), "tc: {:?}", x.text_content);
        assert!(x.text_content.contains("dos"), "tc: {:?}", x.text_content);
    }

    #[test]
    fn event_bubbles_to_document_cubre_click_y_teclas_no_focus() {
        assert!(event_bubbles_to_document("click"));
        assert!(event_bubbles_to_document("keydown"));
        assert!(event_bubbles_to_document("change"));
        // focus/blur NO bubblean en spec.
        assert!(!event_bubbles_to_document("focus"));
        assert!(!event_bubbles_to_document("blur"));
        assert!(!event_bubbles_to_document("scroll"));
    }

    #[test]
    fn click_en_elemento_bubblea_al_document_listener() {
        // Event delegation: el listener vive en document, no en el botón.
        let mut m = model_con_script("console.log('boot')");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "btn".into(),
            tag_name: "button".into(),
            text_content: "go".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "document.addEventListener('click', \
                function(e){ console.log('deleg:' + e.target.id); })",
        )
        .expect("e");
        dispatch_js_event(&mut m, "btn", "click", 0);
        let rt = m.tabs[0].js.as_ref().expect("rt");
        assert!(
            rt.stdout().contains("deleg:btn"),
            "el listener de document debió correr con target=btn; stdout: {:?}",
            rt.stdout()
        );
    }

    #[test]
    fn document_prevent_default_cancela_el_fallback_del_link() {
        // Un handler delegado en document que llama preventDefault debe
        // reflejarse en result.default_prevented (lo usa el chrome para no
        // navegar el `<a>`).
        let mut m = model_con_script("console.log('boot')");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "lnk".into(),
            tag_name: "a".into(),
            text_content: "x".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "document.addEventListener('click', function(e){ e.preventDefault(); })",
        )
        .expect("e");
        let (result, _) = dispatch_js_event(&mut m, "lnk", "click", 0);
        assert!(result.default_prevented, "preventDefault del document debe contar");
    }

    #[test]
    fn dispatch_js_event_corre_handler_y_acumula_logs() {
        let mut m = model_con_script("/* sin scripts */ console.log('boot')");
        // El runtime ya existe gracias al script de boot. Registramos
        // manualmente un elemento + handler antes del dispatch.
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "btn".into(),
            tag_name: "button".into(),
            text_content: "click me".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "document.getElementById('btn').onclick = \
                function(){ console.log('clicked') }",
        )
        .expect("e");
        let logs0 = m.tabs[0].js_summary.logs;
        dispatch_js_event(&mut m, "btn", "click", 0);
        assert!(
            m.tabs[0].js_summary.logs > logs0,
            "esperaba logs nuevos tras dispatch — logs: {}",
            m.tabs[0].js_summary.logs
        );
        let rt = m.tabs[0].js.as_ref().expect("rt");
        assert!(rt.stdout().contains("clicked"), "stdout: {:?}", rt.stdout());
    }

    #[test]
    fn run_scripts_aplica_text_content_mutations_al_box_tree() {
        // Un script de carga muta textContent — el box_tree debe
        // reflejarlo cuando el chrome chequea las hojas de texto.
        let mut t = TabState::new("about:test".into());
        t.title = "T".into();
        t.url = "about:test".into();
        t.box_tree = Some(parse(
            r#"<body><h1 id="hero">viejo</h1></body>"#,
        ));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some(
                "document.getElementById('hero').textContent = 'nuevo'".into(),
            ),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        let bt = t.box_tree.as_ref().expect("box_tree");
        let mut found_new = false;
        let mut found_old = false;
        bt.walk(|b| {
            if b.text.as_deref() == Some("nuevo") { found_new = true; }
            if b.text.as_deref() == Some("viejo") { found_old = true; }
        });
        assert!(found_new, "esperaba ver 'nuevo' tras la mutación");
        assert!(!found_old, "'viejo' debería haberse reemplazado");
    }

    #[test]
    fn dispatch_event_aplica_mutaciones_post_click() {
        // Handler de click muta textContent — al despachar el click, el
        // box_tree debe quedar actualizado.
        let mut m = model_con_script("/* boot */");
        // El runtime existe (boot lo creó). Registramos un elemento +
        // handler que muta textContent del mismo elemento.
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "out".into(),
            tag_name: "div".into(),
            text_content: "antes".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "document.getElementById('out').onclick = function(){ \
                document.getElementById('out').textContent = 'después'; \
             }",
        )
        .expect("e");
        // Reemplazo manual del box_tree para tener un nodo con
        // element_id='out' que pueda mutarse.
        m.tabs[0].box_tree = Some(parse(
            r#"<body><div id="out">antes</div></body>"#,
        ));
        dispatch_js_event(&mut m, "out", "click", 0);
        let bt = m.tabs[0].box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.text.as_deref() == Some("después") {
                found = true;
            }
        });
        assert!(found, "el handler debió mutar 'antes' a 'después'");
    }

    // ============= Fase 7.42 — Page Visibility =============

    #[test]
    fn switch_active_tab_marca_hidden_la_vieja_y_dispatcha() {
        // Tab 0 con runtime + handler visibilitychange. Tab 1 sin runtime
        // (about:blank). SelectTab(1) debería marcar tab[0] como hidden y
        // disparar el handler.
        let mut m = model_con_script(
            "var got = null; \
             window.addEventListener('visibilitychange', function() { \
                got = document.visibilityState; \
             });",
        );
        m.tabs.push(TabState::new("about:tab2".into()));
        // Disparo SelectTab(1) — usa el helper directamente, no el msg.
        switch_active_tab(&mut m, 1);
        assert_eq!(m.active, 1);
        // El handler de tab[0] debe haber visto el cambio a 'hidden'.
        let v = m.tabs[0].js.as_mut().expect("rt").eval("got").expect("e");
        assert_eq!(v, puriy_js::JsValue::String("hidden".into()));
        let v = m.tabs[0]
            .js
            .as_mut()
            .expect("rt")
            .eval("document.hidden")
            .expect("e");
        assert_eq!(v, puriy_js::JsValue::Bool(true));
    }

    #[test]
    fn switch_active_tab_marca_visible_la_nueva() {
        // Toggle ida-y-vuelta sobre el mismo tab con runtime: el handler
        // ve hidden cuando dejamos de ser activos y visible cuando volvemos.
        let mut m = model_con_script(
            "var states = []; \
             window.addEventListener('visibilitychange', function() { \
                states.push(document.visibilityState); \
             });",
        );
        m.tabs.push(TabState::new("about:tab2".into()));
        switch_active_tab(&mut m, 1); // tab 0 → hidden
        switch_active_tab(&mut m, 0); // tab 0 → visible
        let v = m.tabs[0]
            .js
            .as_mut()
            .expect("rt")
            .eval("states.join(',')")
            .expect("e");
        assert_eq!(v, puriy_js::JsValue::String("hidden,visible".into()));
    }

    // ============= Fase 7.41 — beforeunload =============

    #[test]
    fn start_load_dispara_beforeunload_en_window() {
        // Modelo con runtime + handler de beforeunload que setea flag.
        let mut m = model_con_script(
            "var beforeRan = false; \
             window.addEventListener('beforeunload', function() { beforeRan = true; });",
        );
        // Verifica que el handler todavía no corrió.
        let v = m.tabs[0]
            .js
            .as_mut()
            .expect("rt")
            .eval("beforeRan")
            .expect("e");
        assert_eq!(v, puriy_js::JsValue::Bool(false));
        // Start_load dispatcha beforeunload antes de pisar la URL.
        // El runtime cambia al cargar (porque start_load no destruye el
        // runtime hasta Loaded), así que el flag debe ser visible justo
        // después de start_load.
        let h: Handle<Msg> = Handle::for_test();
        start_load(&mut m, "about:test2".into(), false, &h);
        let v = m.tabs[0]
            .js
            .as_mut()
            .expect("rt")
            .eval("beforeRan")
            .expect("e");
        assert_eq!(v, puriy_js::JsValue::Bool(true));
    }

    // ============= Fase 7.39 — window events =============

    #[test]
    fn dispatch_window_event_scroll_corre_listener_y_ve_scroll_y_actual() {
        // Setup: el script registra un listener que muta el DOM con el
        // scrollY actual cuando dispara 'scroll'.
        let mut m = model_con_script(
            "window.addEventListener('scroll', function() { \
                document.getElementById('out').textContent = String(window.scrollY); \
             });",
        );
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "out".into(),
            tag_name: "div".into(),
            text_content: "0".into(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("set_elements");
        m.tabs[0].box_tree = Some(parse(r#"<body><div id="out">0</div></body>"#));
        // Simulamos un scroll a 150px y dispatcheamos directamente.
        m.tabs[0].scroll_y = 150.0;
        let t = &mut m.tabs[0];
        let (r, _pending) = dispatch_window_js_event_on_tab(t, "scroll", 0);
        assert_eq!(r.count, 1);
        // Verifica que el handler vio scrollY=150 mutando el DOM.
        let bt = m.tabs[0].box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.text.as_deref() == Some("150") {
                found = true;
            }
        });
        assert!(found, "el handler debió ver scrollY=150 y mutar a '150'");
    }

    #[test]
    fn dispatch_window_event_load_corre_window_onload() {
        let mut m = model_con_script("var ran = false; window.onload = function(){ ran = true; };");
        let t = &mut m.tabs[0];
        let (r, _pending) = dispatch_window_js_event_on_tab(t, "load", 0);
        assert_eq!(r.count, 1);
        let v = m.tabs[0].js.as_mut().expect("rt").eval("ran").expect("e");
        assert_eq!(v, puriy_js::JsValue::Bool(true));
    }

    #[test]
    fn resize_actualiza_viewport_y_corre_listener() {
        // El listener de 'resize' lee window.innerWidth y lo escribe al DOM.
        let mut m = model_con_script(
            "window.addEventListener('resize', function() { \
                document.getElementById('out').textContent = String(window.innerWidth); \
             });",
        );
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "out".into(),
            tag_name: "div".into(),
            text_content: "0".into(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("set_elements");
        m.tabs[0].box_tree = Some(parse(r#"<body><div id="out">0</div></body>"#));
        // Msg::Resize debe: (1) set_viewport(800,600) ANTES del dispatch,
        // (2) disparar 'resize' → el handler ve innerWidth=800.
        let h: Handle<Msg> = Handle::for_test();
        let m = Puriy::update(m, Msg::Resize(800, 600), &h);
        let bt = m.tabs[0].box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.text.as_deref() == Some("800") {
                found = true;
            }
        });
        assert!(found, "el handler de resize debió ver innerWidth=800 y mutar a '800'");
    }

    #[test]
    fn on_resize_devuelve_msg_resize() {
        let m = model_con_script("/* boot */");
        assert!(matches!(
            Puriy::on_resize(&m, 640, 480),
            Some(Msg::Resize(640, 480))
        ));
    }

    #[test]
    fn on_scale_factor_devuelve_msg_scale_factor() {
        let m = model_con_script("/* boot */");
        assert!(matches!(
            Puriy::on_scale_factor(&m, 2.0),
            Some(Msg::ScaleFactor(s)) if s == 2.0
        ));
    }

    #[test]
    fn scale_factor_actualiza_devicePixelRatio_y_corre_listener() {
        // El listener de 'resize' lee window.devicePixelRatio y lo escribe al
        // DOM (los browsers disparan 'resize' al cambiar el DPI).
        let mut m = model_con_script(
            "window.addEventListener('resize', function() { \
                document.getElementById('out').textContent = String(window.devicePixelRatio); \
             });",
        );
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "out".into(),
            tag_name: "div".into(),
            text_content: "1".into(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("set_elements");
        m.tabs[0].box_tree = Some(parse(r#"<body><div id="out">1</div></body>"#));
        // Msg::ScaleFactor(2.0) debe: (1) set_device_pixel_ratio(2) ANTES del
        // dispatch, (2) disparar 'resize' → el handler ve devicePixelRatio=2.
        let h: Handle<Msg> = Handle::for_test();
        let m = Puriy::update(m, Msg::ScaleFactor(2.0), &h);
        let bt = m.tabs[0].box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.text.as_deref() == Some("2") {
                found = true;
            }
        });
        assert!(found, "el handler de resize debió ver devicePixelRatio=2 y mutar a '2'");
    }