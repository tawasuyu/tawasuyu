#[allow(unused_imports)] use super::*;
#[allow(unused_imports)] use super::super::*;
#[allow(unused_imports)] use llimphi_raster::kurbo::{Cap, Join};
#[allow(unused_imports)] use llimphi_raster::peniko::{Brush, Extend};



    #[test]
    fn apply_remove_child_quita_box_node() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(
            r#"<body><ul id="list"><li id="a">a</li><li id="b">b</li></ul></body>"#,
        ));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[
            puriy_js::ElementSnapshot {
                id: "list".into(),
                tag_name: "ul".into(),
                text_content: String::new(),
                class_list: Vec::new(),
                value: None,
                parent_id: None,
                dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
            },
            puriy_js::ElementSnapshot {
                id: "a".into(),
                tag_name: "li".into(),
                text_content: "a".into(),
                class_list: Vec::new(),
                value: None,
                parent_id: Some("list".into()),
                dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
            },
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('list').removeChild(document.getElementById('a'))",
        )
        .expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        // El <li id=a> ya no debería existir; el <li id=b> sí.
        let mut a_exists = false;
        let mut b_exists = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("a") {
                a_exists = true;
            }
            if b.element_id.as_deref() == Some("b") {
                b_exists = true;
            }
        });
        assert!(!a_exists);
        assert!(b_exists);
    }

    // ============= Fase 7.14 — insertBefore + herencia de estilos =============

    #[test]
    fn apply_insert_before_pone_child_antes_del_ref() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(
            r#"<body><ul id="list"><li id="a">a</li><li id="b">b</li></ul></body>"#,
        ));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[
            puriy_js::ElementSnapshot {
                id: "list".into(),
                tag_name: "ul".into(),
                text_content: String::new(),
                class_list: Vec::new(),
                value: None,
                parent_id: None,
                dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
            },
            puriy_js::ElementSnapshot {
                id: "a".into(),
                tag_name: "li".into(),
                text_content: "a".into(),
                class_list: Vec::new(),
                value: None,
                parent_id: Some("list".into()),
                dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
            },
        ])
        .expect("e");
        rt.eval(
            "var li = document.createElement('li'); \
             li.id = 'mid'; \
             li.textContent = 'mid'; \
             document.getElementById('list').insertBefore(li, document.getElementById('a'));",
        )
        .expect("e");
        apply_dom_mutations(t);
        // Orden esperado en BoxTree: mid, a, b.
        let bt = t.box_tree.as_ref().expect("bt");
        let mut order: Vec<String> = Vec::new();
        bt.walk(|b| {
            if b.tag.as_deref() == Some("li") {
                if let Some(id) = &b.element_id {
                    order.push(id.clone());
                }
            }
        });
        assert_eq!(order, vec!["mid", "a", "b"]);
    }

    #[test]
    fn apply_insert_before_ref_inexistente_hace_append() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><ul id="list"><li id="a">a</li></ul></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[
            puriy_js::ElementSnapshot {
                id: "list".into(),
                tag_name: "ul".into(),
                text_content: String::new(),
                class_list: Vec::new(),
                value: None,
                parent_id: None,
                dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
            },
        ])
        .expect("e");
        // El ref_id "fantasma" no existe — el chrome cae a append.
        // Simulamos la mutación manualmente (saltea las validaciones JS).
        rt.eval("globalThis.__puriy_dirty.push({id:'list',kind:'insertBefore',value:'li\u{001D}nuevo\u{001D}x\u{001D}\u{001D}\u{001D}fantasma'})")
            .expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        let mut order: Vec<String> = Vec::new();
        bt.walk(|b| {
            if b.tag.as_deref() == Some("li") {
                if let Some(id) = &b.element_id {
                    order.push(id.clone());
                }
            }
        });
        // 'nuevo' debe estar después de 'a' porque cae a append.
        assert_eq!(order, vec!["a", "nuevo"]);
    }

    #[test]
    fn append_child_hereda_color_y_font_size_del_parent() {
        // Parent <div id=p> con style="color:red;font-size:24px" tiene
        // esos valores en su BoxNode. Un <li> sintético appendChild
        // debería heredar color rojo + font_size 24, en lugar de los
        // defaults negros 16px.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(
            r#"<body><div id="p" style="color: red; font-size: 24px"></div></body>"#,
        ));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "p".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval(
            "var s = document.createElement('span'); \
             s.id = 'k'; \
             s.textContent = 'hola'; \
             document.getElementById('p').appendChild(s);",
        )
        .expect("e");
        apply_dom_mutations(t);
        // El <span id=k> sintético debe tener color y font_size del padre.
        let bt = t.box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("k") {
                assert!(
                    (b.font_size - 24.0).abs() < 0.01,
                    "font_size esperado 24, got {}",
                    b.font_size
                );
                // color: red (255,0,0) en el formato Color de engine.
                assert_eq!((b.color.r, b.color.g, b.color.b), (255, 0, 0), "color esperado red");
                found = true;
            }
        });
        assert!(found);
    }

    #[test]
    fn append_child_y_textcontent_post_insercion() {
        // appendChild + mutación de textContent después de insertar
        // deberían actualizar el text leaf del BoxNode sintético.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><div id="p"></div></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "p".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        // textContent inicial via el payload del appendChild.
        rt.eval(
            "var d = document.createElement('span'); \
             d.id = 'item1'; \
             d.textContent = 'inicial'; \
             document.getElementById('p').appendChild(d); \
             document.getElementById('item1').textContent = 'actualizado';",
        )
        .expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        // El text leaf bajo el span#item1 debe ser 'actualizado'.
        let mut got = String::new();
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("item1") {
                if let Some(c) = b.children.first() {
                    if let Some(t) = &c.text {
                        got = t.clone();
                    }
                }
            }
        });
        assert_eq!(got, "actualizado");
    }

    #[test]
    fn apply_dataset_remove_mutation_quita_la_key() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><div id="x" data-role="main">y</div></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "x".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: vec![("role".into(), "main".into())],
            attributes: vec![("data-role".into(), "main".into())],
            dfs_index: 0,
        }])
        .expect("e");
        rt.eval("delete document.getElementById('x').dataset.role")
            .expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        let mut still_there = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                if b.dataset().iter().any(|(k, _)| *k == "role") {
                    still_there = true;
                }
            }
        });
        assert!(!still_there, "data-role no debería existir tras el delete");
    }

    // ============= Fase 7.16 — attributes genéricos =============

    #[test]
    fn collect_element_snapshots_pobla_attributes_completo() {
        let tree = parse(
            r#"<body><a id="nav" href="https://tawasuyu.net" aria-current="page" data-track="hero" rel="noopener">x</a></body>"#,
        );
        let snaps = collect_element_snapshots(&tree);
        let s = snaps.iter().find(|s| s.id == "nav").expect("found");
        // attributes incluye TODOS los attrs (data-*, aria-*, href, rel, id).
        assert!(s.attributes.iter().any(|(k, v)| k == "href" && v == "https://tawasuyu.net"));
        assert!(s.attributes.iter().any(|(k, v)| k == "aria-current" && v == "page"));
        assert!(s.attributes.iter().any(|(k, v)| k == "data-track" && v == "hero"));
        assert!(s.attributes.iter().any(|(k, v)| k == "rel" && v == "noopener"));
        // dataset sigue filtrando sólo data-* sin prefijo.
        assert!(s.dataset.iter().any(|(k, v)| k == "track" && v == "hero"));
        assert!(s.dataset.iter().all(|(k, _)| !k.starts_with("data-")));
    }

    #[test]
    fn apply_attr_mutation_actualiza_box_tree() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><div id="x">y</div></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "x".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('x').setAttribute('aria-label', 'main')")
            .expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("x")
                && b.attributes.iter().any(|(k, v)| k == "aria-label" && v == "main")
            {
                found = true;
            }
        });
        assert!(found, "setAttribute debería poblar attributes en el BoxTree");
    }

    #[test]
    fn apply_attr_remove_mutation_quita_la_key() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><a id="x" href="/old">y</a></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "x".into(),
            tag_name: "a".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: vec![("href".into(), "/old".into())],
            dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('x').removeAttribute('href')").expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        let mut still = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("x")
                && b.attributes.iter().any(|(k, _)| k == "href")
            {
                still = true;
            }
        });
        assert!(!still, "removeAttribute debe quitar href del BoxTree");
    }

    // ============= Fase 7.18 — focus()/blur() chrome-side =============

    #[test]
    fn apply_focus_mutation_setea_focused_input_si_es_input_slot() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><input id="user" /><input id="pw" /></body>"#));
        // Pre-pueblo inputs_element_ids como lo hace Msg::Loaded (orden DFS).
        t.inputs.push(TextInputState::new());
        t.inputs.push(TextInputState::new());
        t.inputs_element_ids = vec![Some("user".into()), Some("pw".into())];
        t.focused_input = None;
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[
            puriy_js::ElementSnapshot {
                id: "user".into(),
                tag_name: "input".into(),
                text_content: String::new(),
                class_list: Vec::new(),
                value: Some(String::new()),
                parent_id: None,
                dataset: Vec::new(),
                attributes: Vec::new(),
                dfs_index: 0,
            },
            puriy_js::ElementSnapshot {
                id: "pw".into(),
                tag_name: "input".into(),
                text_content: String::new(),
                class_list: Vec::new(),
                value: Some(String::new()),
                parent_id: None,
                dataset: Vec::new(),
                attributes: Vec::new(),
                dfs_index: 0,
            },
        ])
        .expect("e");
        rt.eval("document.getElementById('pw').focus()").expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.focused_input, Some(1), "el focus en 'pw' (slot 1) debió moverse");
    }

    #[test]
    fn apply_focus_mutation_sobre_no_input_no_afecta_focused_input() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><button id="btn">x</button></body>"#));
        t.focused_input = None;
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "btn".into(),
            tag_name: "button".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('btn').focus()").expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.focused_input, None, "focus en un button no afecta el cursor");
    }

    #[test]
    fn apply_blur_mutation_limpia_focused_input_si_era_el_actual() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><input id="user" /></body>"#));
        t.inputs.push(TextInputState::new());
        t.inputs_element_ids = vec![Some("user".into())];
        t.focused_input = Some(0);
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "user".into(),
            tag_name: "input".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: Some(String::new()),
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('user').blur()").expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.focused_input, None);
    }

    // ============= Fase 7.19 — text node sintético =============

    #[test]
    fn apply_append_text_node_inserta_text_leaf_sin_tag() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><div id="parent"></div></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "parent".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("e");
        rt.eval(
            "var p = document.getElementById('parent'); \
             p.append(document.createTextNode('Hola mundo'));",
        )
        .expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("parent") {
                for c in &b.children {
                    if c.tag.is_none() && c.text.as_deref() == Some("Hola mundo") {
                        found = true;
                    }
                }
            }
        });
        assert!(found, "parent debe tener text leaf 'Hola mundo' como hijo");
    }

    // ============= Fase 7.24 — scrollIntoView chrome-side =============

    #[test]
    fn apply_scroll_into_view_setea_scroll_y_por_dfs_order() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        // Tree con varios elementos para que la posición DFS varíe.
        t.box_tree = Some(parse(
            r#"<body><div id="top">top</div><div id="mid">mid</div><div id="bot">bottom</div></body>"#,
        ));
        t.scroll_y = 0.0;
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[
            puriy_js::ElementSnapshot {
                id: "bot".into(),
                tag_name: "div".into(),
                text_content: String::new(),
                class_list: Vec::new(),
                value: None,
                parent_id: None,
                dataset: Vec::new(),
                attributes: Vec::new(),
                dfs_index: 0,
            },
        ])
        .expect("e");
        rt.eval("document.getElementById('bot').scrollIntoView()").expect("e");
        apply_dom_mutations(t);
        // bot está más profundo en el DFS pre-order que top/mid → scroll_y > 0.
        assert!(t.scroll_y > 0.0, "scroll_y debería avanzar hacia el elemento (got {})", t.scroll_y);
    }

    #[test]
    fn apply_scroll_into_view_id_inexistente_no_modifica_scroll() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><div id="x">x</div></body>"#));
        t.scroll_y = 42.0;
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "x".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("e");
        // Disparamos scrollIntoView contra un id que NO está en el box_tree.
        // El JS sí publica la mutación (no valida); el chrome la silencia.
        rt.eval(
            "globalThis.__puriy_dirty.push({id: 'fantasma', kind: 'scrollIntoView', value: ''});",
        )
        .expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.scroll_y, 42.0, "scroll no debe moverse para id inexistente");
    }

    // ============= Fase 7.26 — window.scrollTo aplicado al chrome =============

    #[test]
    fn apply_scroll_mutation_actualiza_scroll_y_del_tab() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body></body>"#));
        t.scroll_y = 0.0;
        let rt = t.js.as_mut().expect("rt");
        rt.eval("scrollTo(0, 250)").expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.scroll_y, 250.0);
    }

    #[test]
    fn apply_scroll_mutation_clampea_a_no_negativo() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body></body>"#));
        t.scroll_y = 100.0;
        let rt = t.js.as_mut().expect("rt");
        rt.eval("scrollTo(0, -50)").expect("e");
        apply_dom_mutations(t);
        assert_eq!(t.scroll_y, 0.0);
    }