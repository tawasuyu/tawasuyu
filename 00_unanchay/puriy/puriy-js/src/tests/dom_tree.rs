//! Tests del DOM: sibling, insertBefore, createElement, appendChild, remove, tagName, focus/blur, attributes, outerHTML, createTextNode, append, prepend, replaceWith, before, after, cloneNode, contains.
    use super::*;

    // ============= Fase 7.14 — sibling + insertBefore =============

    #[test]
    fn previous_next_element_sibling_recorren_hermanos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("a", "li", "p"),
            snap_with_parent("b", "li", "p"),
            snap_with_parent("c", "li", "p"),
        ])
        .expect("e");
        let v = rt
            .eval("document.getElementById('b').previousElementSibling.id")
            .expect("e");
        assert_eq!(v, JsValue::String("a".into()));
        let v = rt
            .eval("document.getElementById('b').nextElementSibling.id")
            .expect("e");
        assert_eq!(v, JsValue::String("c".into()));
        // Bordes: primer y último.
        let v = rt
            .eval("document.getElementById('a').previousElementSibling")
            .expect("e");
        assert_eq!(v, JsValue::Null);
        let v = rt
            .eval("document.getElementById('c').nextElementSibling")
            .expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn sibling_devuelve_null_sin_parent() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("solo", "div", "")]).expect("e");
        let v = rt
            .eval("document.getElementById('solo').previousElementSibling")
            .expect("e");
        assert_eq!(v, JsValue::Null);
        let v = rt
            .eval("document.getElementById('solo').nextElementSibling")
            .expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn insert_before_publica_mutacion_con_ref_id() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("ref", "li", "p"),
        ])
        .expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var li = document.createElement('li'); \
             li.id = 'nuevo'; \
             document.getElementById('p').insertBefore(li, document.getElementById('ref'));",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].id, "p");
        assert_eq!(muts[0].kind, "insertBefore");
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts.len(), 6);
        assert_eq!(parts[0], "li");
        assert_eq!(parts[1], "nuevo");
        assert_eq!(parts[5], "ref"); // ref_id
    }

    #[test]
    fn insert_before_null_equivale_a_appendchild() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "ul", "")]).expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var li = document.createElement('li'); \
             document.getElementById('p').insertBefore(li, null);",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        // null refChild → fallback a appendChild.
        assert_eq!(muts[0].kind, "appendChild");
    }

    #[test]
    fn children_for_of_funciona() {
        // Fase 7.15 — children devuelve un Array nativo, así que
        // for...of (via Array.prototype[Symbol.iterator]) funciona.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("a", "li", "p"),
            snap_with_parent("b", "li", "p"),
        ])
        .expect("e");
        rt.eval(
            "var out = ''; \
             for (var c of document.getElementById('p').children) { out += c.id; } \
             out;",
        )
        .map(|v| match v {
            JsValue::String(s) => assert_eq!(s, "ab"),
            other => panic!("expected String, got {:?}", other),
        })
        .expect("e");
    }

    #[test]
    fn children_array_methods_funcionan() {
        // children es Array → soporta forEach/map/filter/some/etc.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("a", "li", "p"),
            snap_with_parent("b", "li", "p"),
        ])
        .expect("e");
        let v = rt
            .eval(
                "document.getElementById('p').children.map(function(c){ return c.id; }).join('+')",
            )
            .expect("e");
        assert_eq!(v, JsValue::String("a+b".into()));
    }

    #[test]
    fn replace_child_publica_insert_before_seguido_de_remove() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("old", "li", "p"),
        ])
        .expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var n = document.createElement('li'); \
             n.id = 'new'; \
             document.getElementById('p').replaceChild(n, document.getElementById('old'));",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        // Esperamos 2 mutaciones: insertBefore + removeChild.
        assert_eq!(muts.len(), 2);
        assert_eq!(muts[0].kind, "insertBefore");
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "new");
        assert_eq!(parts[5], "old"); // ref_id
        assert_eq!(muts[1].kind, "removeChild");
        assert_eq!(muts[1].value, "old");
    }

    #[test]
    fn get_attribute_id_class_value_data() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[ElementSnapshot {
            id: "x".into(),
            tag_name: "input".into(),
            text_content: String::new(),
            class_list: vec!["a".into(), "b".into()],
            value: Some("hola".into()),
            parent_id: None,
            dataset: vec![("role".into(), "main".into())],
            attributes: vec![("data-role".into(), "main".into())],
            dfs_index: 0,
        }])
        .expect("e");
        let v = rt.eval("document.getElementById('x').getAttribute('id')").expect("e");
        assert_eq!(v, JsValue::String("x".into()));
        let v = rt.eval("document.getElementById('x').getAttribute('class')").expect("e");
        assert_eq!(v, JsValue::String("a b".into()));
        let v = rt.eval("document.getElementById('x').getAttribute('value')").expect("e");
        assert_eq!(v, JsValue::String("hola".into()));
        let v = rt.eval("document.getElementById('x').getAttribute('data-role')").expect("e");
        assert_eq!(v, JsValue::String("main".into()));
        let v = rt.eval("document.getElementById('x').getAttribute('nada')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn set_attribute_data_publica_dataset_mutation() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('x').setAttribute('data-foo-bar', 'val')")
            .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "dataset:foo-bar");
        assert_eq!(muts[0].value, "val");
        // El getter reflexivo devuelve el value seteado.
        let v = rt.eval("document.getElementById('x').getAttribute('data-foo-bar')").expect("e");
        assert_eq!(v, JsValue::String("val".into()));
    }

    #[test]
    fn set_attribute_id_reindexa() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("old", "div", "")]).expect("e");
        rt.eval("document.getElementById('old').setAttribute('id', 'nuevo')").expect("e");
        // getElementById('nuevo') ahora encuentra el elemento; 'old' es null.
        let v = rt.eval("document.getElementById('nuevo').tagName").expect("e");
        assert_eq!(v, JsValue::String("DIV".into()));
        let v = rt.eval("document.getElementById('old')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn has_attribute_devuelve_true_solo_si_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_class("x", "div", "", "foo")]).expect("e");
        assert_eq!(
            rt.eval("document.getElementById('x').hasAttribute('class')").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(
            rt.eval("document.getElementById('x').hasAttribute('id')").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(
            rt.eval("document.getElementById('x').hasAttribute('data-foo')").expect("e"),
            JsValue::Bool(false)
        );
    }

    #[test]
    fn remove_attribute_data_publica_dataset_remove() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[ElementSnapshot {
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
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('x').removeAttribute('data-role')").expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "dataset-remove:role");
        let v = rt.eval("document.getElementById('x').getAttribute('data-role')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    // Fase 7.16 — attrs genéricos (aria-*, href, src...) ahora se publican
    // como `attr:<name>` y se reflejan tanto en _attributes_store como en
    // el BoxNode al aplicar la mutación.
    #[test]
    fn set_attribute_generico_publica_attr_mutation() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('x').setAttribute('aria-label', 'main nav')")
            .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "attr:aria-label");
        assert_eq!(muts[0].value, "main nav");
        // El getter reflexivo devuelve el value seteado.
        let v = rt.eval("document.getElementById('x').getAttribute('aria-label')").expect("e");
        assert_eq!(v, JsValue::String("main nav".into()));
        // hasAttribute lo reconoce.
        let v = rt.eval("document.getElementById('x').hasAttribute('aria-label')").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn get_attribute_lee_attribute_initial_del_snapshot() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_attrs("x", "a", &[
            ("href", "https://tawasuyu.net"),
            ("aria-current", "page"),
            ("title", "ir a inicio"),
        ])])
        .expect("e");
        let v = rt.eval("document.getElementById('x').getAttribute('href')").expect("e");
        assert_eq!(v, JsValue::String("https://tawasuyu.net".into()));
        let v = rt.eval("document.getElementById('x').getAttribute('aria-current')").expect("e");
        assert_eq!(v, JsValue::String("page".into()));
        // hasAttribute true para los presentes, false para los ausentes.
        assert_eq!(
            rt.eval("document.getElementById('x').hasAttribute('title')").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(
            rt.eval("document.getElementById('x').hasAttribute('rel')").expect("e"),
            JsValue::Bool(false)
        );
        // Name uppercased en JS se normaliza a lowercase para matchear el store.
        let v = rt.eval("document.getElementById('x').getAttribute('HREF')").expect("e");
        assert_eq!(v, JsValue::String("https://tawasuyu.net".into()));
    }

    #[test]
    fn remove_attribute_generico_publica_attr_remove() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_attrs("x", "a", &[("href", "https://x.io")])])
            .expect("e");
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('x').removeAttribute('href')").expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "attr-remove:href");
        let v = rt.eval("document.getElementById('x').getAttribute('href')").expect("e");
        assert_eq!(v, JsValue::Null);
        let v = rt.eval("document.getElementById('x').hasAttribute('href')").expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

    #[test]
    fn replace_child_falla_si_old_no_es_hijo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p1", "ul", ""),
            snap_with_parent("a", "li", "p1"),
            snap("p2", "ul", ""),
        ])
        .expect("e");
        let res = rt.eval(
            "var n = document.createElement('li'); \
             try { document.getElementById('p2').replaceChild(n, document.getElementById('a')); 'ok' } \
             catch (e) { 'err' }",
        );
        assert_eq!(res.expect("e"), JsValue::String("err".into()));
    }

    #[test]
    fn insert_before_falla_si_ref_no_es_hijo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p1", "ul", ""),
            snap_with_parent("a", "li", "p1"),
            snap("p2", "ul", ""),
        ])
        .expect("e");
        let res = rt.eval(
            "var li = document.createElement('li'); \
             try { document.getElementById('p2').insertBefore(li, document.getElementById('a')); 'ok' } \
             catch (e) { 'err' }",
        );
        assert_eq!(res.expect("e"), JsValue::String("err".into()));
    }

    #[test]
    fn once_capture_listener_tambien_se_borra() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "div", ""),
            snap_with_parent("c", "span", "p"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('p').addEventListener('click', \
                function(){ console.log('cap') }, { capture: true, once: true });",
        )
        .expect("e");
        rt.dispatch_event("c", "click", None).expect("d1");
        rt.dispatch_event("c", "click", None).expect("d2");
        assert_eq!(rt.stdout(), "cap\n");
    }

    // ============= Fase 7.12 — createElement + appendChild/remove =============

    #[test]
    fn create_element_devuelve_handle_sintetico() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        let v = rt
            .eval("var el = document.createElement('li'); el.tagName")
            .expect("e");
        assert_eq!(v, JsValue::String("LI".into()));
        // _synthetic flag presente
        let v = rt.eval("el._synthetic").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        // id auto-generado
        let v = rt
            .eval("el.id.indexOf('__synth_') === 0")
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn create_element_se_registra_en_elements() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var el = document.createElement('div')").expect("e");
        // Buscable via getElementById usando el synth id.
        let v = rt
            .eval("document.getElementById(el.id) === el")
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn append_child_publica_mutacion_con_payload_delim() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("list", "ul", "")]).expect("e");
        assert!(rt.drain_dom_mutations().is_empty());
        rt.eval(
            "var li = document.createElement('li'); \
             li.textContent = 'hola'; \
             document.getElementById('list').appendChild(li);",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].id, "list");
        assert_eq!(muts[0].kind, "appendChild");
        // Payload campos: tag, id, textContent, classes, value (separados
        // por U+001D). Parser básico abajo.
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0], "li");
        assert!(parts[1].starts_with("__synth_"));
        assert_eq!(parts[2], "hola");
        assert_eq!(parts[3], "");
        assert_eq!(parts[4], "");
    }

    #[test]
    fn append_child_falla_si_child_no_es_sintetico() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "div", ""), snap("b", "div", "")])
            .expect("e");
        let res = rt.eval(
            "try { document.getElementById('a').appendChild(document.getElementById('b')); 'ok' } \
             catch (e) { 'err' }",
        );
        assert_eq!(res.expect("e"), JsValue::String("err".into()));
    }

    #[test]
    fn append_child_falla_si_ya_fue_insertado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p1", "ul", ""), snap("p2", "ul", "")])
            .expect("e");
        let res = rt.eval(
            "var li = document.createElement('li'); \
             document.getElementById('p1').appendChild(li); \
             try { document.getElementById('p2').appendChild(li); 'ok' } \
             catch (e) { 'err' }",
        );
        assert_eq!(res.expect("e"), JsValue::String("err".into()));
    }

    #[test]
    fn remove_child_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("c", "li", "p"),
        ])
        .expect("e");
        assert!(rt.drain_dom_mutations().is_empty());
        rt.eval(
            "document.getElementById('p').removeChild(document.getElementById('c'))",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].id, "p");
        assert_eq!(muts[0].kind, "removeChild");
        assert_eq!(muts[0].value, "c");
    }

    #[test]
    fn el_remove_publica_mutacion_contra_parent() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("c", "li", "p"),
        ])
        .expect("e");
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('c').remove()").expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        // remove() publica contra el parent, no contra sí mismo.
        assert_eq!(muts[0].id, "p");
        assert_eq!(muts[0].kind, "removeChild");
        assert_eq!(muts[0].value, "c");
    }

    #[test]
    fn append_child_con_id_user_set_usa_ese_id() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "div", "")]).expect("e");
        rt.eval(
            "var d = document.createElement('div'); \
             d.id = 'modal'; \
             d._classList = ['big','center']; \
             d._value = ''; \
             document.getElementById('p').appendChild(d);",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        // El id en payload es 'modal' (user-set), no el synth_id.
        assert_eq!(parts[1], "modal");
        assert_eq!(parts[3], "big center");
    }

    #[test]
    fn dataset_inexistente_devuelve_undefined() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        let v = rt
            .eval("document.getElementById('x').dataset.nada")
            .expect("e");
        assert_eq!(v, JsValue::Undefined);
    }

    #[test]
    fn event_phase_refleja_la_etapa_actual() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').addEventListener('click', \
                function(e){ console.log('cap:' + e.eventPhase) }, true); \
             document.getElementById('btn').onclick = function(e){ \
                console.log('target:' + e.eventPhase) \
            }; \
             document.getElementById('outer').onclick = function(e){ \
                console.log('bubble:' + e.eventPhase) \
            };",
        )
        .expect("e");
        rt.dispatch_event("btn", "click", None).expect("d");
        assert!(rt.stdout().contains("cap:1"), "stdout: {:?}", rt.stdout());
        assert!(rt.stdout().contains("target:2"), "stdout: {:?}", rt.stdout());
        assert!(rt.stdout().contains("bubble:3"), "stdout: {:?}", rt.stdout());
    }

    // ============= Fase 7.17 — tagName UPPERCASE / matches / closest / hasAttributes =============

    #[test]
    fn tag_name_devuelve_uppercase() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", ""), snap("y", "input", "")])
            .expect("e");
        let v = rt.eval("document.getElementById('x').tagName").expect("e");
        assert_eq!(v, JsValue::String("DIV".into()));
        let v = rt.eval("document.getElementById('y').tagName").expect("e");
        assert_eq!(v, JsValue::String("INPUT".into()));
    }

    #[test]
    fn node_name_es_alias_de_tag_name() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "section", "")]).expect("e");
        let v = rt.eval("document.getElementById('x').nodeName").expect("e");
        assert_eq!(v, JsValue::String("SECTION".into()));
    }

    #[test]
    fn query_selector_por_tag_sigue_matcheando_post_uppercase() {
        // Aunque tagName devuelva UPPERCASE, el querySelector internamente
        // compara contra _tagName lowercase — los selectores no necesitan
        // case-change para seguir funcionando.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("hero", "section", "")]).expect("e");
        let v = rt.eval("document.querySelector('section').id").expect("e");
        assert_eq!(v, JsValue::String("hero".into()));
    }

    #[test]
    fn matches_simple_id_class_tag() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "button", "")]).expect("e");
        rt.eval("document.getElementById('x').className = 'btn primary'").expect("e");
        let cases = &[
            ("#x", true),
            ("#z", false),
            (".btn", true),
            (".primary", true),
            (".missing", false),
            ("button", true),
            ("div", false),
        ];
        for (sel, expected) in cases {
            let v = rt
                .eval(&format!("document.getElementById('x').matches('{}')", sel))
                .expect("e");
            assert_eq!(v, JsValue::Bool(*expected), "selector {}", sel);
        }
    }

    #[test]
    fn matches_compound_tag_class_id_attr() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_attrs("x", "a", &[
            ("href", "/about"),
            ("aria-current", "page"),
        ])])
        .expect("e");
        rt.eval("document.getElementById('x').className = 'nav-link'")
            .expect("e");
        // Compound: tag + class + id + [attr=value]
        let v = rt
            .eval(r#"document.getElementById('x').matches('a.nav-link#x[href="/about"]')"#)
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
        // [attr] sin value — sólo presencia.
        let v = rt
            .eval(r#"document.getElementById('x').matches('[aria-current]')"#)
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
        // Falla si una sola parte no matchea.
        let v = rt
            .eval(r#"document.getElementById('x').matches('a.nav-link[href="/otro"]')"#)
            .expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

    #[test]
    fn matches_rechaza_combinadores_y_pseudoclases() {
        // Spec del subset: si el selector tiene combinador o `:`, devuelve
        // false silenciosamente (en vez de crash o falso positivo).
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_class("x", "div", "", "foo")]).expect("e");
        let cases = &[".foo div", "div > .foo", "div + p", "div ~ p", ".foo:hover"];
        for sel in cases {
            let v = rt
                .eval(&format!("document.getElementById('x').matches('{}')", sel))
                .expect("e");
            assert_eq!(v, JsValue::Bool(false), "selector {}", sel);
        }
    }

    #[test]
    fn closest_walka_ancestros_hasta_matchear() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("modal", "div", ""),
            snap_with_parent("body", "section", "modal"),
            snap_with_parent("btn", "button", "body"),
        ])
        .expect("e");
        // self matchea (closest incluye self).
        let v = rt.eval("document.getElementById('btn').closest('button').id").expect("e");
        assert_eq!(v, JsValue::String("btn".into()));
        // Sube hasta el ancestro.
        let v = rt.eval("document.getElementById('btn').closest('#modal').id").expect("e");
        assert_eq!(v, JsValue::String("modal".into()));
        // No matchea ningún ancestro.
        let v = rt.eval("document.getElementById('btn').closest('.inexistente')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn has_attributes_devuelve_true_si_hay_algun_attr() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("con_id", "div", ""),
            snap_with_attrs("solo_attr", "a", &[("href", "/x")]),
        ])
        .expect("e");
        let v = rt.eval("document.getElementById('con_id').hasAttributes()").expect("e");
        assert_eq!(v, JsValue::Bool(true), "tiene id → true");
        let v = rt.eval("document.getElementById('solo_attr').hasAttributes()").expect("e");
        assert_eq!(v, JsValue::Bool(true), "tiene attrs → true");
    }

    // ============= Fase 7.18 — focus()/blur() chrome-side, attributes, outerHTML =============

    #[test]
    fn focus_publica_mutacion_focus_y_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("inp", "input", "")]).expect("e");
        rt.eval(
            "document.getElementById('inp').addEventListener('focus', \
                function() { console.log('focused') });",
        )
        .expect("e");
        rt.eval("document.getElementById('inp').focus()").expect("e");
        // Handler corrió Y se publicó mutación focus para el chrome.
        assert_eq!(rt.stdout(), "focused\n");
        let muts = rt.drain_dom_mutations();
        assert!(muts.iter().any(|m| m.id == "inp" && m.kind == "focus"));
    }

    #[test]
    fn blur_publica_mutacion_blur() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("inp", "input", "")]).expect("e");
        rt.eval("document.getElementById('inp').blur()").expect("e");
        let muts = rt.drain_dom_mutations();
        assert!(muts.iter().any(|m| m.id == "inp" && m.kind == "blur"));
    }

    #[test]
    fn attributes_enumera_id_class_value_dataset_y_genericos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[ElementSnapshot {
            id: "x".into(),
            tag_name: "a".into(),
            text_content: String::new(),
            class_list: vec!["btn".into(), "primary".into()],
            value: None,
            parent_id: None,
            dataset: vec![("track".into(), "hero".into())],
            attributes: vec![
                ("data-track".into(), "hero".into()),
                ("href".into(), "/x".into()),
                ("aria-current".into(), "page".into()),
            ],
            dfs_index: 0,
        }])
        .expect("e");
        let v = rt.eval("document.getElementById('x').attributes.length").expect("e");
        // id, class, data-track, href, aria-current = 5.
        assert_eq!(v, JsValue::Number(5.0));
        // Verificamos forma de cada entry — {name, value}.
        let v = rt
            .eval("document.getElementById('x').attributes[0].name")
            .expect("e");
        assert_eq!(v, JsValue::String("id".into()));
        // attributes es iterable con for...of (JS array nativo).
        let v = rt
            .eval(
                "var names = []; \
                 for (var a of document.getElementById('x').attributes) names.push(a.name); \
                 names.indexOf('href') >= 0 && names.indexOf('aria-current') >= 0",
            )
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn attributes_no_duplica_si_attributes_store_tiene_data_o_id() {
        // Si el snapshot pobla ambos _dataset_store Y _attributes_store
        // con la misma key, attributes debe devolver una sola entry (la
        // del dataset; el _attributes_store skippea las que ya cubrió
        // por rama especial).
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_dataset("x", "div", &[("role", "main")])])
            .expect("e");
        let v = rt
            .eval(
                "var dups = 0; \
                 for (var a of document.getElementById('x').attributes) \
                     if (a.name === 'data-role') dups++; \
                 dups",
            )
            .expect("e");
        assert_eq!(v, JsValue::Number(1.0));
    }

    #[test]
    fn outer_html_serializa_elemento_con_attrs_y_text() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[ElementSnapshot {
            id: "x".into(),
            tag_name: "a".into(),
            text_content: "Inicio".into(),
            class_list: vec!["btn".into()],
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: vec![("href".into(), "/x".into())],
            dfs_index: 0,
        }])
        .expect("e");
        let v = rt.eval("document.getElementById('x').outerHTML").expect("e");
        let JsValue::String(s) = v else { panic!("expected string") };
        // El orden de attrs sigue id, class, [value], data-*, otros.
        assert!(s.starts_with("<a "), "got: {s}");
        assert!(s.contains(r#"id="x""#), "got: {s}");
        assert!(s.contains(r#"class="btn""#), "got: {s}");
        assert!(s.contains(r#"href="/x""#), "got: {s}");
        assert!(s.ends_with(">Inicio</a>"), "got: {s}");
    }

    #[test]
    fn outer_html_void_tag_no_lleva_cierre() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[ElementSnapshot {
            id: "i".into(),
            tag_name: "img".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: vec![("src".into(), "/foo.png".into())],
            dfs_index: 0,
        }])
        .expect("e");
        let v = rt.eval("document.getElementById('i').outerHTML").expect("e");
        assert_eq!(v, JsValue::String(r#"<img id="i" src="/foo.png">"#.into()));
    }

    #[test]
    fn outer_html_escapa_quotes_y_lt_en_attr_y_text() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        // Después del set, el outerHTML debe escapar caracteres especiales.
        rt.eval("document.getElementById('x').setAttribute('title', 'a\"b<c')")
            .expect("e");
        rt.eval("document.getElementById('x').textContent = '<b>&</b>'")
            .expect("e");
        let v = rt.eval("document.getElementById('x').outerHTML").expect("e");
        let JsValue::String(s) = v else { panic!("expected string") };
        assert!(s.contains(r#"title="a&quot;b&lt;c""#), "got: {s}");
        assert!(s.contains("&lt;b&gt;&amp;&lt;/b&gt;"), "got: {s}");
    }

    // ============= Fase 7.19 — createTextNode + append/prepend =============

    #[test]
    fn create_text_node_devuelve_handle_sintetico_con_text_y_tag_vacio() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        let v = rt
            .eval("var t = document.createTextNode('Hola'); t._textContent")
            .expect("e");
        assert_eq!(v, JsValue::String("Hola".into()));
        let v = rt.eval("t._isText").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("t._tagName").expect("e");
        assert_eq!(v, JsValue::String("".into()));
        let v = rt.eval("t._synthetic").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn append_acepta_mezcla_de_elementos_y_strings() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("parent", "div", "")]).expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var p = document.getElementById('parent'); \
             var child = document.createElement('span'); \
             p.append(child, ' texto suelto', document.createElement('em'));",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        // 3 mutaciones de appendChild — span, text node, em.
        let appends: Vec<_> = muts.iter().filter(|m| m.kind == "appendChild").collect();
        assert_eq!(appends.len(), 3);
        // El 2do payload empieza con tag vacío (text node).
        let p2 = &appends[1].value;
        assert!(p2.starts_with('\u{001D}'), "text node payload empieza con sep (tag vacío): {p2:?}");
        // Args null/undefined se silencian.
        rt.drain_dom_mutations();
        rt.eval("p.append(null, undefined)").expect("e");
        assert!(rt.drain_dom_mutations().is_empty());
    }

    #[test]
    fn prepend_invierte_orden_via_insert_before() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("parent", "div", ""), snap_with_parent("existing", "p", "parent")])
            .expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var p = document.getElementById('parent'); \
             var a = document.createElement('li'); a.id = 'a'; \
             var b = document.createElement('li'); b.id = 'b'; \
             p.prepend(a, b);",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        // Las dos inserciones van como insertBefore (hay firstElementChild).
        let inserts: Vec<_> = muts.iter().filter(|m| m.kind == "insertBefore").collect();
        assert_eq!(inserts.len(), 2);
    }

    #[test]
    fn prepend_sin_first_element_child_cae_a_append() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("parent", "div", "")]).expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var p = document.getElementById('parent'); \
             p.prepend(document.createElement('span'));",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        // Sin firstElementChild cae a appendChild.
        assert!(muts.iter().any(|m| m.kind == "appendChild"));
    }

    // ============= Fase 7.20 — replaceWith + before + after =============

    #[test]
    fn replace_with_inserta_y_remueve_el_original() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "div", ""), snap_with_parent("old", "span", "p")])
            .expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var o = document.getElementById('old'); \
             o.replaceWith(document.createElement('section'), ' suelto');",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let inserts: Vec<_> = muts.iter().filter(|m| m.kind == "insertBefore").collect();
        let removes: Vec<_> = muts.iter().filter(|m| m.kind == "removeChild").collect();
        assert_eq!(inserts.len(), 2);
        assert_eq!(removes.len(), 1);
        assert_eq!(removes[0].value, "old");
    }

    #[test]
    fn before_inserta_siblings_antes_del_elemento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "div", ""), snap_with_parent("center", "span", "p")])
            .expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var c = document.getElementById('center'); \
             c.before('hola ', document.createElement('em'));",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let inserts: Vec<_> = muts.iter().filter(|m| m.kind == "insertBefore").collect();
        assert_eq!(inserts.len(), 2);
        assert_eq!(muts.iter().filter(|m| m.kind == "removeChild").count(), 0);
    }

    #[test]
    fn after_sin_next_sibling_cae_a_append_child() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "div", ""), snap_with_parent("last", "span", "p")])
            .expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var l = document.getElementById('last'); \
             l.after(document.createElement('hr'));",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert!(muts.iter().any(|m| m.kind == "appendChild"));
    }

    #[test]
    fn before_after_replace_with_sin_parent_no_op() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("root", "div", "")]).expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var r = document.getElementById('root'); \
             r.before('x'); r.after('y'); r.replaceWith('z');",
        )
        .expect("e");
        assert!(rt.drain_dom_mutations().is_empty());
    }

    // ============= Fase 7.21 — cloneNode + contains =============

    #[test]
    fn clone_node_copia_tag_class_text_attrs() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[ElementSnapshot {
            id: "src".into(),
            tag_name: "a".into(),
            text_content: "click".into(),
            class_list: vec!["btn".into(), "primary".into()],
            value: None,
            parent_id: None,
            dataset: vec![("track".into(), "hero".into())],
            attributes: vec![
                ("data-track".into(), "hero".into()),
                ("href".into(), "/x".into()),
            ],
            dfs_index: 0,
        }])
        .expect("e");
        rt.eval("var c = document.getElementById('src').cloneNode(false)").expect("e");
        let v = rt.eval("c._tagName").expect("e");
        assert_eq!(v, JsValue::String("a".into()));
        let v = rt.eval("c._textContent").expect("e");
        assert_eq!(v, JsValue::String("click".into()));
        let v = rt.eval("c._classList.join(',')").expect("e");
        assert_eq!(v, JsValue::String("btn,primary".into()));
        let v = rt.eval("c._dataset_store.track").expect("e");
        assert_eq!(v, JsValue::String("hero".into()));
        let v = rt.eval("c._attributes_store.href").expect("e");
        assert_eq!(v, JsValue::String("/x".into()));
        // Clone tiene id NUEVO (synth_), no el del original.
        let v = rt.eval("c.id !== 'src' && c.id.indexOf('__synth_') === 0").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        // Clone es synthetic + no insertado (listo para appendChild).
        let v = rt.eval("c._synthetic && !c._inserted").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn clone_node_de_text_node_crea_otro_text_node() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        let v = rt
            .eval(
                "var t = document.createTextNode('Hola'); \
                 var c = t.cloneNode(true); c._textContent",
            )
            .expect("e");
        assert_eq!(v, JsValue::String("Hola".into()));
        let v = rt.eval("c._isText").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn contains_self_devuelve_true() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "div", "")]).expect("e");
        let v = rt
            .eval("var a = document.getElementById('a'); a.contains(a)")
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

