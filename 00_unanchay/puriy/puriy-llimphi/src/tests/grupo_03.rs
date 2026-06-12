#[allow(unused_imports)] use super::*;
#[allow(unused_imports)] use super::super::*;
#[allow(unused_imports)] use llimphi_raster::kurbo::{Cap, Join};
#[allow(unused_imports)] use llimphi_raster::peniko::{Brush, Extend};



    #[test]
    fn current_viewport_refleja_resize_y_scale() {
        // Fase 7.175 — el engine resuelve @media contra este viewport.
        let m = model_con_script("/* x */");
        let h: Handle<Msg> = Handle::for_test();
        let m = Puriy::update(m, Msg::Resize(900, 500), &h);
        let _m = Puriy::update(m, Msg::ScaleFactor(2.0), &h);
        let vp = current_viewport();
        assert_eq!(vp.width, 900.0);
        assert_eq!(vp.height, 500.0);
        assert_eq!(vp.dpr, 2.0);
    }

    #[test]
    fn media_queries_se_resuelven_contra_viewport_y_dpr() {
        // Fase 7.174 — el chrome evalúa matchMedia contra su viewport REAL.
        // Conducimos viewport y DPR por Msg para no depender del thread-local
        // (que otros tests del mismo hilo podrían haber mutado).
        let m = model_con_script(
            "globalThis.__wide = matchMedia('(min-width: 600px)'); \
             globalThis.__huge = matchMedia('(min-width: 1200px)'); \
             globalThis.__hidpi = matchMedia('(min-resolution: 2dppx)');",
        );
        let h: Handle<Msg> = Handle::for_test();
        // Viewport 1000×700 @ dpr 1 → wide sí, huge no, hidpi no.
        let mut m = Puriy::update(m, Msg::Resize(1000, 700), &h);
        {
            let rt = m.tabs[0].js.as_mut().expect("rt");
            assert_eq!(rt.eval("__wide.matches").expect("e"), puriy_js::JsValue::Bool(true));
            assert_eq!(rt.eval("__huge.matches").expect("e"), puriy_js::JsValue::Bool(false));
            assert_eq!(rt.eval("__hidpi.matches").expect("e"), puriy_js::JsValue::Bool(false));
        }
        // Subimos el DPR a 2 → la query de resolution flipea a true.
        let mut m = Puriy::update(m, Msg::ScaleFactor(2.0), &h);
        {
            let rt = m.tabs[0].js.as_mut().expect("rt");
            assert_eq!(rt.eval("__hidpi.matches").expect("e"), puriy_js::JsValue::Bool(true));
        }
        let _ = m;
    }

    #[test]
    fn dispatch_window_event_sin_runtime_es_no_op() {
        // Tab sin runtime — no debe panic.
        let mut t = TabState::new("about:blank".into());
        assert!(t.js.is_none());
        let (r, pending) = dispatch_window_js_event_on_tab(&mut t, "scroll", 0);
        assert_eq!(r.count, 0);
        assert!(pending.is_empty());
    }

    #[test]
    fn tick_aplica_mutaciones_de_settimeout() {
        let mut m = model_con_script("/* boot */");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "clock".into(),
            tag_name: "span".into(),
            text_content: "00:00".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.set_now_ms(0).expect("now");
        rt.eval(
            "setTimeout(function(){ \
                document.getElementById('clock').textContent = '10:00'; \
             }, 50)",
        )
        .expect("e");
        m.tabs[0].box_tree = Some(parse(
            r#"<body><span id="clock">00:00</span></body>"#,
        ));
        tick_js_runtimes(&mut m, 100);
        let bt = m.tabs[0].box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.text.as_deref() == Some("10:00") {
                found = true;
            }
        });
        assert!(found, "tick debió aplicar la mutación del setTimeout");
    }

    #[test]
    fn apply_style_color_actualiza_box_tree_post_script() {
        let mut t = TabState::new("about:test".into());
        t.title = "T".into();
        t.url = "about:test".into();
        t.box_tree = Some(parse(r#"<body><h1 id="h">hola</h1></body>"#));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some("document.getElementById('h').style.color = '#ff0000'".into()),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        let bt = t.box_tree.as_ref().expect("bt");
        let mut red_leaf = false;
        bt.walk(|b| {
            if b.text.as_deref() == Some("hola") && b.color.r == 255 && b.color.g == 0 && b.color.b == 0 {
                red_leaf = true;
            }
        });
        assert!(red_leaf, "el text leaf debió quedar rojo");
    }

    #[test]
    fn apply_style_display_none_oculta_post_dispatch() {
        let mut m = model_con_script("/* boot */");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "panel".into(),
            tag_name: "div".into(),
            text_content: "".into(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval(
            "document.getElementById('panel').onclick = function(){ \
                document.getElementById('panel').style.display = 'none'; \
             }",
        )
        .expect("e");
        m.tabs[0].box_tree = Some(parse(r#"<body><div id="panel">x</div></body>"#));
        dispatch_js_event(&mut m, "panel", "click", 0);
        let bt = m.tabs[0].box_tree.as_ref().expect("bt");
        let mut hidden = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("panel") {
                if matches!(b.display, puriy_engine::Display::None) {
                    hidden = true;
                }
            }
        });
        assert!(hidden);
    }

    #[test]
    fn collect_element_snapshots_propaga_class_list() {
        let tree = parse(r#"<div><h1 id="hero" class="title big">x</h1></div>"#);
        let snaps = collect_element_snapshots(&tree);
        assert_eq!(snaps.len(), 1);
        assert!(snaps[0].class_list.contains(&"title".to_string()));
        assert!(snaps[0].class_list.contains(&"big".to_string()));
    }

    #[test]
    fn dispatch_event_devuelve_default_prevented_cuando_corresponde() {
        let mut m = model_con_script("/* boot */");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "a".into(),
            tag_name: "a".into(),
            text_content: "link".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval(
            "document.getElementById('a').onclick = function(e){ e.preventDefault(); }",
        )
        .expect("e");
        let (r, _) = dispatch_js_event(&mut m, "a", "click", 0);
        assert!(r.default_prevented);
        assert_eq!(r.count, 1);
    }

    #[test]
    fn dispatch_keydown_focus_blur_change_son_event_types_validos() {
        // Sanity: el harness JS acepta cualquier event_type — no está
        // restringido a 'click'. Esto destraba Fase 7.7.
        let mut m = model_con_script("/* boot */");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "i".into(),
            tag_name: "input".into(),
            text_content: "".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval(
            "var el = document.getElementById('i'); \
             el.addEventListener('keydown', function(e){ console.log('K:'+e.type) }); \
             el.addEventListener('focus',   function(e){ console.log('F:'+e.type) }); \
             el.addEventListener('blur',    function(e){ console.log('B:'+e.type) }); \
             el.addEventListener('change',  function(e){ console.log('C:'+e.type) });",
        )
        .expect("e");
        dispatch_js_event(&mut m, "i", "keydown", 0);
        dispatch_js_event(&mut m, "i", "focus", 0);
        dispatch_js_event(&mut m, "i", "blur", 0);
        dispatch_js_event(&mut m, "i", "change", 0);
        let rt = m.tabs[0].js.as_ref().expect("rt");
        let out = rt.stdout();
        assert!(out.contains("K:keydown"), "stdout: {out:?}");
        assert!(out.contains("F:focus"), "stdout: {out:?}");
        assert!(out.contains("B:blur"), "stdout: {out:?}");
        assert!(out.contains("C:change"), "stdout: {out:?}");
    }

    #[test]
    fn dispatch_event_sin_prevent_default_devuelve_false() {
        let mut m = model_con_script("/* boot */");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "a".into(),
            tag_name: "a".into(),
            text_content: "link".into(), class_list: Vec::new(), value: None, parent_id: None, dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('a').onclick = function(){ /* nada */ }")
            .expect("e");
        let (r, _) = dispatch_js_event(&mut m, "a", "click", 0);
        assert!(!r.default_prevented);
        assert_eq!(r.count, 1);
    }

    #[test]
    fn dispatch_sobre_id_sin_handler_no_panic() {
        let mut m = model_con_script("console.log('boot')");
        // No registramos ningún elemento — el dispatch va al vacío.
        dispatch_js_event(&mut m, "fantasma", "click", 0);
        // Si llegamos acá sin panic, OK.
        let rt = m.tabs[0].js.as_ref().expect("rt");
        // stdout sigue siendo sólo el "boot" del script inicial.
        assert!(rt.stdout().contains("boot"));
    }

    // ============= Fase 7.9 — event.key + Element.value =============

    #[test]
    fn named_key_name_mapea_teclas_comunes() {
        use llimphi_ui::NamedKey;
        assert_eq!(named_key_name(&NamedKey::Enter), "Enter");
        assert_eq!(named_key_name(&NamedKey::Escape), "Escape");
        assert_eq!(named_key_name(&NamedKey::ArrowLeft), "ArrowLeft");
        assert_eq!(named_key_name(&NamedKey::ArrowRight), "ArrowRight");
        assert_eq!(named_key_name(&NamedKey::Tab), "Tab");
        assert_eq!(named_key_name(&NamedKey::Backspace), "Backspace");
        assert_eq!(named_key_name(&NamedKey::F5), "F5");
    }

    #[test]
    fn key_event_to_init_extrae_caracter_y_modifiers() {
        use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers};
        let e = KeyEvent {
            key: Key::Character("a".into()),
            state: KeyState::Pressed,
            text: Some("a".into()),
            modifiers: Modifiers {
                shift: true,
                ctrl: false,
                alt: false,
                meta: false,
            },
            repeat: false,
        };
        let init = key_event_to_init(&e);
        assert_eq!(init.key.as_deref(), Some("a"));
        assert_eq!(init.code.as_deref(), Some("a"));
        assert_eq!(init.shift_key, Some(true));
        assert_eq!(init.ctrl_key, Some(false));
    }

    #[test]
    fn key_event_to_init_mapea_named_keys() {
        use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers, NamedKey};
        let e = KeyEvent {
            key: Key::Named(NamedKey::ArrowDown),
            state: KeyState::Pressed,
            text: None,
            modifiers: Modifiers::default(),
            repeat: false,
        };
        let init = key_event_to_init(&e);
        assert_eq!(init.key.as_deref(), Some("ArrowDown"));
    }

    #[test]
    fn collect_element_snapshots_value_de_input_lleva_input_initial() {
        let tree = parse(r#"<body><input id="email" value="hola@x.com"></body>"#);
        let snaps = collect_element_snapshots(&tree);
        let s = snaps.iter().find(|s| s.id == "email").expect("found");
        assert_eq!(s.value.as_deref(), Some("hola@x.com"));
    }

    #[test]
    fn collect_element_snapshots_value_de_select_lleva_option_seleccionado() {
        let tree = parse(
            r#"<body><select id="lang">
                <option value="es">Español</option>
                <option value="en" selected>English</option>
            </select></body>"#,
        );
        let snaps = collect_element_snapshots(&tree);
        let s = snaps.iter().find(|s| s.id == "lang").expect("found");
        assert_eq!(s.value.as_deref(), Some("en"));
    }

    #[test]
    fn collect_element_snapshots_value_es_none_para_div() {
        let tree = parse(r#"<body><div id="x">hola</div></body>"#);
        let snaps = collect_element_snapshots(&tree);
        let s = snaps.iter().find(|s| s.id == "x").expect("found");
        assert_eq!(s.value, None);
    }

    #[test]
    fn apply_value_mutation_actualiza_text_input_state() {
        // JS setea el.value = "nuevo" — apply_dom_mutations debe
        // propagarlo al TextInputState del slot correspondiente.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><input id="x" value="viejo"></body>"#));
        let mut s = TextInputState::new();
        s.set_text("viejo".to_string());
        t.inputs = vec![s];
        t.inputs_element_ids = vec![Some("x".into())];
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "x".into(),
            tag_name: "input".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: Some("viejo".into()),
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('x').value = 'nuevo'")
            .expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.inputs[0].text(), "nuevo");
    }

    #[test]
    fn clipboard_write_text_emite_set_system_clipboard() {
        // navigator.clipboard.writeText publica una mutación kind:'clipboard';
        // apply_dom_mutations debe traducirla a Msg::SetSystemClipboard para
        // que el update loop la empuje al portapapeles real (Fase 7.176).
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        let rt = t.js.as_mut().expect("rt");
        rt.eval("navigator.clipboard.writeText('copiado por JS')")
            .expect("e");
        let out = apply_dom_mutations(t);
        let writes: Vec<&str> = out
            .iter()
            .filter_map(|msg| match msg {
                Msg::SetSystemClipboard(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(writes, vec!["copiado por JS"]);
    }

    #[test]
    fn eventsource_mutation_emite_es_open_y_close() {
        // El bootstrap de EventSource publica una mutación `kind:'eventsource'`
        // al construir y al cerrar; apply_dom_mutations las traduce a
        // Msg::EsOpen/EsClose (sin abrir red — eso es del worker).
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.js.as_mut().unwrap().eval("var es = new EventSource('http://x/sse');").expect("e");
        let out = apply_dom_mutations(t);
        assert!(
            out.iter().any(|msg| matches!(msg, Msg::EsOpen { es_id: 1, url, .. } if url == "http://x/sse")),
            "no se emitió EsOpen"
        );
        t.js.as_mut().unwrap().eval("es.close();").expect("e");
        let out2 = apply_dom_mutations(t);
        assert!(
            out2.iter().any(|msg| matches!(msg, Msg::EsClose { es_id: 1, .. })),
            "no se emitió EsClose"
        );
    }

    #[test]
    fn es_dispatch_msg_entrega_evento_al_listener() {
        // Msg::EsDispatch (lo que manda el worker) debe llegar al onmessage del
        // EventSource correcto, vía el host method rt.es_dispatch.
        let mut m = model_con_script(
            "var got = null; var es = new EventSource('http://x/sse'); \
             es.onmessage = function(e) { got = e.data + ':' + e.lastEventId; };",
        );
        let es_id = match m.tabs[0].js.as_mut().unwrap().eval("es._id").unwrap() {
            puriy_js::JsValue::Number(n) => n as u32,
            other => panic!("es._id no es número: {other:?}"),
        };
        let (tab, gen) = (m.tabs[0].id, m.tabs[0].gen);
        let h: Handle<Msg> = Handle::for_test();
        let m = Puriy::update(
            m,
            Msg::EsDispatch {
                tab,
                gen,
                es_id,
                kind: "message".into(),
                event_type: "message".into(),
                data: "hola".into(),
                last_id: "9".into(),
            },
            &h,
        );
        let mut m = m;
        let got = m.tabs[0].js.as_mut().unwrap().eval("got").expect("e");
        assert_eq!(got, puriy_js::JsValue::String("hola:9".into()));
    }

    #[test]
    fn run_scripts_siembra_el_portapapeles_del_sistema() {
        // Con system_clipboard = Some(...), un readText() de un script inicial
        // ve lo que el usuario tiene copiado afuera, no la cadena vacía.
        let mut t = TabState::new("about:blank".into());
        t.box_tree = Some(parse("<p>x</p>"));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some(
                "var leido = ''; navigator.clipboard.readText().then(function(x){ leido = x; });"
                    .to_string(),
            ),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, Some("desde el sistema"));
        let rt = t.js.as_mut().expect("rt");
        assert_eq!(
            rt.eval("leido").expect("e"),
            puriy_js::JsValue::String("desde el sistema".into())
        );
    }

    #[test]
    fn apply_value_mutation_actualiza_select_state() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(
            r#"<body><select id="lang">
                <option value="es">Español</option>
                <option value="en">English</option>
            </select></body>"#,
        ));
        t.selects = vec![SelectState { selected: 0, open: false }];
        t.selects_element_ids = vec![Some("lang".into())];
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "lang".into(),
            tag_name: "select".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: Some("es".into()),
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('lang').value = 'en'")
            .expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.selects[0].selected, 1);
    }

    #[test]
    fn dispatch_keydown_pasa_key_real_al_handler() {
        use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers, NamedKey};
        let mut m = model_con_script("/* boot */");
        let rt = m.tabs[0].js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "i".into(),
            tag_name: "input".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: Some(String::new()),
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval(
            "document.getElementById('i').onkeydown = function(ev){ \
                console.log(ev.key) \
            }",
        )
        .expect("e");
        let e = KeyEvent {
            key: Key::Named(NamedKey::Enter),
            state: KeyState::Pressed,
            text: None,
            modifiers: Modifiers::default(),
            repeat: false,
        };
        let init = key_event_to_init(&e);
        dispatch_js_event_with_init(&mut m, "i", "keydown", 0, Some(init));
        let rt = m.tabs[0].js.as_ref().expect("rt");
        assert!(rt.stdout().contains("Enter"), "stdout: {:?}", rt.stdout());
    }

    #[test]
    fn select_value_at_devuelve_value_del_option() {
        let tree = parse(
            r#"<body><select id="lang">
                <option value="es">Español</option>
                <option value="en">English</option>
            </select></body>"#,
        );
        let mut m = model_con_script("/* boot */");
        m.tabs[0].box_tree = Some(tree);
        assert_eq!(select_value_at(&m.tabs[0], 0, 1).as_deref(), Some("en"));
        assert_eq!(select_value_at(&m.tabs[0], 0, 0).as_deref(), Some("es"));
        assert_eq!(select_value_at(&m.tabs[0], 99, 0), None);
    }

    // ============= Fase 7.10 — bubbling + input event =============

    #[test]
    fn collect_element_snapshots_pobla_parent_id_directo() {
        // <div id=outer><button id=btn></button></div>
        let tree = parse(r#"<body><div id="outer"><button id="btn">x</button></div></body>"#);
        let snaps = collect_element_snapshots(&tree);
        let outer = snaps.iter().find(|s| s.id == "outer").expect("outer");
        let btn = snaps.iter().find(|s| s.id == "btn").expect("btn");
        assert_eq!(outer.parent_id, None);
        assert_eq!(btn.parent_id.as_deref(), Some("outer"));
    }

    #[test]
    fn collect_element_snapshots_salta_ancestros_sin_id() {
        // <section id=s><div><button id=btn></button></div></section>
        // El <div> sin id no aparece en la cadena de bubbling — btn
        // pasa a tener parent_id = s directamente.
        let tree = parse(
            r#"<body><section id="s"><div><button id="btn">x</button></div></section></body>"#,
        );
        let snaps = collect_element_snapshots(&tree);
        let btn = snaps.iter().find(|s| s.id == "btn").expect("btn");
        assert_eq!(btn.parent_id.as_deref(), Some("s"));
    }

    #[test]
    fn collect_element_snapshots_root_sin_parent() {
        // El elemento del root no debe tener parent_id.
        let tree = parse(r#"<body><div id="root">x</div></body>"#);
        let snaps = collect_element_snapshots(&tree);
        let root = snaps.iter().find(|s| s.id == "root").expect("root");
        assert_eq!(root.parent_id, None);
    }

    #[test]
    fn collect_element_snapshots_tres_niveles_de_anidacion() {
        let tree = parse(
            r#"<body><div id="a"><div id="b"><div id="c">x</div></div></div></body>"#,
        );
        let snaps = collect_element_snapshots(&tree);
        let a = snaps.iter().find(|s| s.id == "a").expect("a");
        let b = snaps.iter().find(|s| s.id == "b").expect("b");
        let c = snaps.iter().find(|s| s.id == "c").expect("c");
        assert_eq!(a.parent_id, None);
        assert_eq!(b.parent_id.as_deref(), Some("a"));
        assert_eq!(c.parent_id.as_deref(), Some("b"));
    }

    // ============= Fase 7.11 — dataset =============

    #[test]
    fn collect_element_snapshots_pobla_dataset() {
        let tree =
            parse(r#"<body><div id="x" data-role="banner" data-id-key="42">x</div></body>"#);
        let snaps = collect_element_snapshots(&tree);
        let s = snaps.iter().find(|s| s.id == "x").expect("found");
        // El suffix preserva case del HTML; el value tal cual.
        assert!(s.dataset.iter().any(|(k, v)| k == "role" && v == "banner"));
        assert!(s.dataset.iter().any(|(k, v)| k == "id-key" && v == "42"));
    }