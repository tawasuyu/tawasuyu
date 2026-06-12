//! Tests del DOM: eventos de elementos, timers, bubbling, capture, dataset, event.key, Element.value, options.once.
    use super::*;

    #[test]
    fn get_element_by_id_devuelve_el_indexado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("hero", "h1", "Hola mundo")]).expect("e");
        let v = rt.eval("document.getElementById('hero').tagName").expect("e");
        // Fase 7.17 — tagName devuelve UPPERCASE (spec del DOM API).
        assert_eq!(v, JsValue::String("H1".into()));
        let v = rt.eval("document.getElementById('hero').textContent").expect("e");
        assert_eq!(v, JsValue::String("Hola mundo".into()));
        let v = rt.eval("document.getElementById('inexistente')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn query_selector_class_busca_por_classlist() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("x", "div", "uno"),
            snap_with_class("y", "div", "dos", "foo"),
        ])
        .expect("e");
        let v = rt.eval("document.querySelector('.foo').id").expect("e");
        assert_eq!(v, JsValue::String("y".into()));
        let v = rt.eval("document.querySelector('.bar')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn query_selector_tag_busca_por_tagname() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("h", "h1", "título"),
            snap("p", "p", "párrafo"),
        ])
        .expect("e");
        let v = rt.eval("document.querySelector('p').id").expect("e");
        assert_eq!(v, JsValue::String("p".into()));
        let v = rt.eval("document.querySelector('h1').id").expect("e");
        assert_eq!(v, JsValue::String("h".into()));
        let v = rt.eval("document.querySelector('span')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn classlist_add_remove_toggle_contains() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_class("x", "div", "", "foo")]).expect("e");
        assert_eq!(
            rt.eval("document.getElementById('x').classList.contains('foo')").expect("e"),
            JsValue::Bool(true)
        );
        rt.eval("document.getElementById('x').classList.add('bar')").expect("e");
        assert_eq!(
            rt.eval("document.getElementById('x').classList.contains('bar')").expect("e"),
            JsValue::Bool(true)
        );
        rt.eval("document.getElementById('x').classList.remove('foo')").expect("e");
        assert_eq!(
            rt.eval("document.getElementById('x').classList.contains('foo')").expect("e"),
            JsValue::Bool(false)
        );
        rt.eval("document.getElementById('x').classList.toggle('baz')").expect("e");
        assert_eq!(
            rt.eval("document.getElementById('x').classList.contains('baz')").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn classlist_publica_mutacion_para_restyle() {
        // Fase 7.184 — add/remove/className publican la mutación 'classList'
        // con la lista COMPLETA de clases para que el chrome recascadee.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_class("x", "div", "", "foo")]).expect("e");
        rt.eval("document.getElementById('x').classList.add('bar')").expect("e");
        let muts = rt.drain_dom_mutations();
        let cl: Vec<_> = muts.iter().filter(|m| m.kind == "classList").collect();
        assert_eq!(cl.len(), 1, "una mutación classList: {muts:?}");
        assert_eq!(cl[0].id, "x");
        assert_eq!(cl[0].value, "foo bar");
        // className setter publica la lista nueva completa.
        rt.eval("document.getElementById('x').className = 'a b'").expect("e");
        let muts = rt.drain_dom_mutations();
        let cl: Vec<_> = muts.iter().filter(|m| m.kind == "classList").collect();
        assert_eq!(cl.last().expect("classList mut").value, "a b");
    }

    #[test]
    fn query_selector_id_consulta_indice() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "contenido")]).expect("e");
        let v = rt.eval("document.querySelector('#x').id").expect("e");
        assert_eq!(v, JsValue::String("x".into()));
        // Selectores no-id siguen devolviendo null en esta fase.
        let v = rt.eval("document.querySelector('.foo')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn add_event_listener_se_registra_y_dispara() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "click me")]).expect("e");
        rt.eval(
            "document.getElementById('btn').addEventListener('click', \
                function(){ console.log('clicked') })",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("dispatch"); let count = r.count;
        assert_eq!(count, 1);
        assert_eq!(rt.stdout(), "clicked\n");
    }

    #[test]
    fn onclick_property_se_dispara_igual_que_listener() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "x")]).expect("e");
        rt.eval(
            "document.getElementById('btn').onclick = function(){ console.log('on') }",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("dispatch"); let count = r.count;
        assert_eq!(count, 1);
        assert_eq!(rt.stdout(), "on\n");
    }

    #[test]
    fn onclick_y_listeners_disparan_ambos() {
        // Si setear `.onclick = fn` Y registrar listener via
        // addEventListener, ambos corren.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "x")]).expect("e");
        rt.eval(
            "var el = document.getElementById('btn'); \
             el.onclick = function(){ console.log('property') }; \
             el.addEventListener('click', function(){ console.log('listener') });",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("dispatch"); let count = r.count;
        assert_eq!(count, 2);
        assert_eq!(rt.stdout(), "property\nlistener\n");
    }

    #[test]
    fn dispatch_sobre_id_inexistente_devuelve_cero() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[]).expect("e");
        let r = rt.dispatch_event("fantasma", "click", None).expect("dispatch"); let count = r.count;
        assert_eq!(count, 0);
    }

    #[test]
    fn remove_event_listener_cancela() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "x")]).expect("e");
        rt.eval(
            "var el = document.getElementById('btn'); \
             var f = function(){ console.log('boom') }; \
             el.addEventListener('click', f); \
             el.removeEventListener('click', f);",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("dispatch"); let count = r.count;
        assert_eq!(count, 0);
        assert!(rt.stdout().is_empty());
    }

    #[test]
    fn error_en_handler_no_aborta_los_siguientes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "x")]).expect("e");
        rt.eval(
            "var el = document.getElementById('btn'); \
             el.addEventListener('click', function(){ throw new Error('boom') }); \
             el.addEventListener('click', function(){ console.log('sigo') });",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("dispatch"); let count = r.count;
        assert_eq!(count, 2);
        assert_eq!(rt.stdout(), "sigo\n");
        assert!(rt.stderr().contains("boom"), "stderr: {:?}", rt.stderr());
    }

    #[test]
    fn set_elements_reset_borra_los_anteriores() {
        // Una página recarga y el snapshot cambia — los elementos
        // viejos no deben sobrevivir.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "div", "uno")]).expect("e");
        assert_eq!(
            rt.eval("!!document.getElementById('a')").expect("e"),
            JsValue::Bool(true)
        );
        // Snapshot nuevo sin "a".
        rt.set_elements(&[snap("b", "div", "dos")]).expect("e");
        assert_eq!(
            rt.eval("document.getElementById('a')").expect("e"),
            JsValue::Null
        );
        assert_eq!(
            rt.eval("document.getElementById('b').textContent").expect("e"),
            JsValue::String("dos".into())
        );
    }

    #[test]
    fn set_text_content_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("hero", "h1", "viejo")]).expect("e");
        // Antes del setter, no hay mutaciones.
        assert!(rt.drain_dom_mutations().is_empty());
        rt.eval("document.getElementById('hero').textContent = 'nuevo'")
            .expect("set");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].id, "hero");
        assert_eq!(muts[0].kind, "text");
        assert_eq!(muts[0].value, "nuevo");
    }

    #[test]
    fn set_inner_html_se_trata_como_text_content() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "a")]).expect("e");
        rt.eval("document.getElementById('x').innerHTML = '<b>raw</b>'")
            .expect("set");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "text");
        assert_eq!(muts[0].value, "<b>raw</b>");
    }

    #[test]
    fn drain_es_idempotente() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "a")]).expect("e");
        rt.eval("document.getElementById('x').textContent = 'b'")
            .expect("e");
        let first = rt.drain_dom_mutations();
        assert_eq!(first.len(), 1);
        let second = rt.drain_dom_mutations();
        assert!(second.is_empty(), "segundo drain debe estar vacío");
    }

    #[test]
    fn multiples_mutaciones_ordenadas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("a", "div", "x"),
            snap("b", "div", "y"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('a').textContent = 'A1'; \
             document.getElementById('b').textContent = 'B1'; \
             document.getElementById('a').textContent = 'A2';",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 3);
        assert_eq!(muts[0].id, "a");
        assert_eq!(muts[0].value, "A1");
        assert_eq!(muts[1].id, "b");
        assert_eq!(muts[1].value, "B1");
        assert_eq!(muts[2].id, "a");
        assert_eq!(muts[2].value, "A2");
    }

    #[test]
    fn text_content_get_devuelve_el_valor_actualizado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "inicial")]).expect("e");
        rt.eval("document.getElementById('x').textContent = 'actualizado'")
            .expect("set");
        let v = rt.eval("document.getElementById('x').textContent").expect("get");
        assert_eq!(v, JsValue::String("actualizado".into()));
    }

    #[test]
    fn set_elements_resetea_el_buffer_dirty() {
        // Si una página recarga, las mutaciones pendientes de la
        // página anterior NO deben filtrarse.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "a")]).expect("e");
        rt.eval("document.getElementById('x').textContent = 'b'")
            .expect("e");
        // Page recarga: nuevo snapshot — el buffer debe quedar vacío.
        rt.set_elements(&[snap("y", "div", "z")]).expect("e2");
        let muts = rt.drain_dom_mutations();
        assert!(muts.is_empty(), "mutación previa fugó: {muts:?}");
    }

    #[test]
    fn set_style_color_publica_mutacion_style() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval("document.getElementById('x').style.color = 'red'")
            .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].id, "x");
        assert_eq!(muts[0].kind, "style:color");
        assert_eq!(muts[0].value, "red");
    }

    #[test]
    fn set_style_camel_case_se_convierte_a_kebab() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval("document.getElementById('x').style.backgroundColor = 'blue'")
            .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "style:background-color");
        assert_eq!(muts[0].value, "blue");
    }

    #[test]
    fn style_get_devuelve_el_valor_seteado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval("document.getElementById('x').style.color = 'green'")
            .expect("e");
        let v = rt.eval("document.getElementById('x').style.color").expect("get");
        assert_eq!(v, JsValue::String("green".into()));
    }

    #[test]
    fn mutacion_con_caracteres_especiales_se_preserva() {
        // RS/US son nuestros delimiters — el value puede contener
        // newlines, comillas, etc. sin romper la decodificación.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval(
            "document.getElementById('x').textContent = 'línea1\\nlínea2\\t\"foo\"'",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].value, "línea1\nlínea2\t\"foo\"");
    }

    #[test]
    fn handler_recibe_event_object_con_type_y_target() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "x")]).expect("e");
        rt.eval(
            "document.getElementById('btn').onclick = function(e){ \
                console.log(e.type + ' ' + e.target.id); \
             }",
        )
        .expect("e");
        rt.dispatch_event("btn", "click", None).expect("dispatch");
        assert_eq!(rt.stdout(), "click btn\n");
    }

    #[test]
    fn prevent_default_lo_reporta_dispatch_result() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "a", "link")]).expect("e");
        rt.eval(
            "document.getElementById('a').onclick = function(e){ e.preventDefault(); }",
        )
        .expect("e");
        let r = rt.dispatch_event("a", "click", None).expect("dispatch");
        assert_eq!(r.count, 1);
        assert!(r.default_prevented, "esperaba default_prevented=true");
    }

    #[test]
    fn stop_propagation_lo_reporta_dispatch_result() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "a", "link")]).expect("e");
        rt.eval(
            "document.getElementById('a').onclick = function(e){ e.stopPropagation(); }",
        )
        .expect("e");
        let r = rt.dispatch_event("a", "click", None).expect("dispatch");
        assert_eq!(r.count, 1);
        assert!(
            r.propagation_stopped,
            "esperaba propagation_stopped=true tras stopPropagation()"
        );
    }

    #[test]
    fn sin_stop_propagation_dispatch_result_lo_marca_falso() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "a", "link")]).expect("e");
        rt.eval("document.getElementById('a').onclick = function(){ /* nada */ }")
            .expect("e");
        let r = rt.dispatch_event("a", "click", None).expect("dispatch");
        assert_eq!(r.count, 1);
        assert!(!r.propagation_stopped);
    }

    #[test]
    fn parse_dispatch_result_acepta_dos_o_tres_campos() {
        // Ruta de elemento: tres campos.
        assert_eq!(
            parse_dispatch_result("2,1,1"),
            DispatchResult { count: 2, default_prevented: true, propagation_stopped: true }
        );
        // Rutas window/document: dos campos → stopped default false.
        assert_eq!(
            parse_dispatch_result("3,0"),
            DispatchResult { count: 3, default_prevented: false, propagation_stopped: false }
        );
    }

    #[test]
    fn sin_prevent_default_dispatch_result_lo_marca_falso() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "a", "link")]).expect("e");
        rt.eval(
            "document.getElementById('a').onclick = function(){ /* no preventDefault */ }",
        )
        .expect("e");
        let r = rt.dispatch_event("a", "click", None).expect("dispatch");
        assert_eq!(r.count, 1);
        assert!(!r.default_prevented);
    }

    #[test]
    fn prevent_default_de_un_handler_no_se_pierde_aunque_otros_no_lo_llamen() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("a", "a", "link")]).expect("e");
        rt.eval(
            "var el = document.getElementById('a'); \
             el.addEventListener('click', function(){ /* nada */ }); \
             el.addEventListener('click', function(e){ e.preventDefault(); }); \
             el.addEventListener('click', function(){ console.log('ran') });",
        )
        .expect("e");
        let r = rt.dispatch_event("a", "click", None).expect("dispatch");
        assert_eq!(r.count, 3, "los 3 listeners deben correr");
        assert!(r.default_prevented);
        assert_eq!(rt.stdout(), "ran\n");
    }

    #[test]
    fn dispatch_result_default_para_id_inexistente() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[]).expect("e");
        let r = rt.dispatch_event("fantasma", "click", None).expect("dispatch");
        assert_eq!(r.count, 0);
        assert!(!r.default_prevented);
    }

    #[test]
    fn handler_puede_registrar_timer_que_se_dispara_despues() {
        // Cadena event → setTimeout: handler hace setTimeout(fn, 50)
        // que el tick subsiguiente dispara.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "x")]).expect("e");
        rt.set_now_ms(0).expect("now");
        rt.eval(
            "document.getElementById('btn').onclick = function(){ \
                setTimeout(function(){ console.log('después') }, 50); \
            }",
        )
        .expect("e");
        rt.dispatch_event("btn", "click", None).expect("dispatch");
        // Aún no se disparó el timer.
        assert!(rt.stdout().is_empty());
        rt.tick(100).expect("tick");
        assert_eq!(rt.stdout(), "después\n");
    }

    #[test]
    fn timers_respetan_now_ms_del_host_no_clock_real() {
        // El host pasa now_ms manualmente — los timers no avanzan con
        // wall clock. Probar que sin tick no hay fire.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval("setTimeout(function(){ console.log('x') }, 1)")
            .expect("e");
        std::thread::sleep(std::time::Duration::from_millis(50));
        // El wall clock avanzó pero __puriy_now_ms NO. Sin tick, no
        // hay fire.
        assert_eq!(rt.pending_timers(), 1);
        assert!(rt.stdout().is_empty());
    }

    // ============= Fase 7.9 — event.key/code + Element.value =============

    #[test]
    fn event_init_keydown_expone_key_y_code_al_handler() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("inp", "input", "")]).expect("e");
        rt.eval(
            "document.getElementById('inp').onkeydown = function(ev){ \
                console.log(ev.key + ':' + ev.code) \
            }",
        )
        .expect("e");
        let init = EventInit {
            key: Some("Enter".into()),
            code: Some("Enter".into()),
            ..Default::default()
        };
        let r = rt.dispatch_event("inp", "keydown", Some(&init)).expect("d");
        assert_eq!(r.count, 1);
        assert_eq!(rt.stdout(), "Enter:Enter\n");
    }

    #[test]
    fn event_init_modifiers_llegan_al_handler() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval(
            "document.getElementById('x').onkeydown = function(ev){ \
                console.log((ev.shiftKey?'S':'-')+(ev.ctrlKey?'C':'-')+(ev.altKey?'A':'-')) \
            }",
        )
        .expect("e");
        let init = EventInit {
            shift_key: Some(true),
            ctrl_key: Some(false),
            alt_key: Some(true),
            ..Default::default()
        };
        rt.dispatch_event("x", "keydown", Some(&init)).expect("d");
        assert_eq!(rt.stdout(), "S-A\n");
    }

    #[test]
    fn event_init_sin_init_no_define_key_code() {
        // El comportamiento Fase 7.7 sigue vivo: si el chrome NO pasa
        // init (o pasa None), los campos viejos del event siguen ahí
        // pero `event.key` queda undefined.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval(
            "document.getElementById('x').onclick = function(ev){ \
                console.log(typeof ev.key) \
            }",
        )
        .expect("e");
        rt.dispatch_event("x", "click", None).expect("d");
        assert_eq!(rt.stdout(), "undefined\n");
    }

    #[test]
    fn event_init_value_sincroniza_el_value_antes_de_handlers() {
        // El chrome pasa value="hola" → handler ve event.target.value
        // === "hola" porque el bootstrap actualiza el._value.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_value("inp", "input", "viejo")])
            .expect("e");
        rt.eval(
            "document.getElementById('inp').onchange = function(ev){ \
                console.log(ev.target.value) \
            }",
        )
        .expect("e");
        let init = EventInit {
            value: Some("nuevo".into()),
            ..Default::default()
        };
        rt.dispatch_event("inp", "change", Some(&init)).expect("d");
        assert_eq!(rt.stdout(), "nuevo\n");
        // Tras el dispatch, el mirror local ya quedó actualizado.
        let v = rt.eval("document.getElementById('inp').value").expect("e");
        assert_eq!(v, JsValue::String("nuevo".into()));
    }

    #[test]
    fn element_value_initial_se_lee_desde_snapshot() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_value("inp", "input", "hola")])
            .expect("e");
        let v = rt.eval("document.getElementById('inp').value").expect("e");
        assert_eq!(v, JsValue::String("hola".into()));
    }

    #[test]
    fn element_value_setter_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_value("inp", "input", "viejo")])
            .expect("e");
        assert!(rt.drain_dom_mutations().is_empty());
        rt.eval("document.getElementById('inp').value = 'nuevo'")
            .expect("set");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].id, "inp");
        assert_eq!(muts[0].kind, "value");
        assert_eq!(muts[0].value, "nuevo");
    }

    #[test]
    fn element_value_sin_snapshot_devuelve_empty() {
        // Si el snapshot vino con value: None (no es un input), el
        // mirror local arranca como "".
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "texto")]).expect("e");
        let v = rt.eval("document.getElementById('x').value").expect("e");
        assert_eq!(v, JsValue::String(String::new()));
    }

    #[test]
    fn event_init_to_js_literal_emite_objeto_o_null() {
        let empty = EventInit::default();
        assert_eq!(empty.to_js_literal(), "null");
        let full = EventInit {
            key: Some("a".into()),
            shift_key: Some(true),
            value: Some("v".into()),
            ..Default::default()
        };
        let lit = full.to_js_literal();
        assert!(lit.starts_with('{') && lit.ends_with('}'));
        assert!(lit.contains("key:\"a\""));
        assert!(lit.contains("shiftKey:true"));
        assert!(lit.contains("value:\"v\""));
    }

    // ============= Fase 7.10 — bubbling DOM =============

    #[test]
    fn bubbling_dispara_handler_del_padre() {
        // <div id=outer><button id=btn></button></div>
        // click en btn debe disparar handler en btn Y en outer.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').onclick = function(e){ \
                console.log('outer:' + e.target.id + ':' + e.currentTarget.id) \
            }; \
             document.getElementById('btn').onclick = function(e){ \
                console.log('btn:' + e.target.id + ':' + e.currentTarget.id) \
            };",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("d");
        assert_eq!(r.count, 2);
        // target permanece fijo a 'btn'; currentTarget cambia al subir.
        assert!(rt.stdout().contains("btn:btn:btn"));
        assert!(rt.stdout().contains("outer:btn:outer"));
    }

    #[test]
    fn stop_propagation_detiene_el_bubble() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').onclick = function(){ \
                console.log('OUTER') \
            }; \
             document.getElementById('btn').onclick = function(e){ \
                console.log('BTN'); e.stopPropagation(); \
            };",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("d");
        // Sólo se disparó el handler de btn; outer NO se llamó.
        assert_eq!(r.count, 1);
        assert_eq!(rt.stdout(), "BTN\n");
    }

    #[test]
    fn bubbling_se_detiene_en_root_sin_parent() {
        // Elemento sin parent_id no debe seguir bubbling.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("solo", "div", "")]).expect("e");
        rt.eval(
            "document.getElementById('solo').onclick = function(){ console.log('hit') }",
        )
        .expect("e");
        let r = rt.dispatch_event("solo", "click", None).expect("d");
        assert_eq!(r.count, 1);
        assert_eq!(rt.stdout(), "hit\n");
    }

    #[test]
    fn bubbling_no_dispara_handlers_de_otro_tipo() {
        // Padre tiene handler de 'mouseover'; dispatch de 'click' al
        // hijo no debe disparar el handler del padre.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').onmouseover = function(){ console.log('over') }; \
             document.getElementById('btn').onclick = function(){ console.log('clicked') };",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("d");
        assert_eq!(r.count, 1);
        assert_eq!(rt.stdout(), "clicked\n");
    }

    #[test]
    fn bubbling_tres_niveles_sube_completo() {
        // <section id=section><div id=outer><button id=btn></button></div></section>
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("section", "section", ""),
            snap_with_parent("outer", "div", "section"),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('section').onclick = function(){ console.log('S') }; \
             document.getElementById('outer').onclick   = function(){ console.log('O') }; \
             document.getElementById('btn').onclick     = function(){ console.log('B') };",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("d");
        assert_eq!(r.count, 3);
        assert_eq!(rt.stdout(), "B\nO\nS\n");
    }

    #[test]
    fn parent_element_resuelve_via_id() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        let v = rt
            .eval("document.getElementById('btn').parentElement.id")
            .expect("e");
        assert_eq!(v, JsValue::String("outer".into()));
        let v = rt
            .eval("document.getElementById('outer').parentElement")
            .expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn bubbling_no_repite_si_hay_ciclo_en_parent_id() {
        // Si el chrome mal-pobló parent_id apuntando a sí mismo,
        // el guard visited rompe el loop antes de agotar fuel.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_parent("x", "div", "x")])
            .expect("e");
        rt.eval(
            "document.getElementById('x').onclick = function(){ console.log('once') }",
        )
        .expect("e");
        let r = rt.dispatch_event("x", "click", None).expect("d");
        assert_eq!(r.count, 1);
        assert_eq!(rt.stdout(), "once\n");
    }

    // ============= Fase 7.11 — capture phase =============

    #[test]
    fn capture_phase_corre_antes_que_bubble() {
        // <outer><inner><btn/></inner></outer>
        // capture listener en outer corre PRIMERO, antes que el handler
        // del target y antes que cualquier bubble.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("inner", "div", "outer"),
            snap_with_parent("btn", "button", "inner"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').addEventListener('click', \
                function(){ console.log('outerCAPTURE') }, true); \
             document.getElementById('inner').addEventListener('click', \
                function(){ console.log('innerCAPTURE') }, {capture:true}); \
             document.getElementById('btn').onclick = function(){ console.log('btnTARGET') }; \
             document.getElementById('outer').onclick = function(){ console.log('outerBUBBLE') };",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("d");
        // Orden esperado: outerCAPTURE → innerCAPTURE → btnTARGET → outerBUBBLE.
        assert_eq!(r.count, 4);
        assert_eq!(
            rt.stdout(),
            "outerCAPTURE\ninnerCAPTURE\nbtnTARGET\nouterBUBBLE\n"
        );
    }

    #[test]
    fn capture_listener_puede_stop_propagation() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').addEventListener('click', \
                function(e){ console.log('CAP'); e.stopPropagation(); }, true); \
             document.getElementById('btn').onclick = function(){ console.log('BTN') };",
        )
        .expect("e");
        let r = rt.dispatch_event("btn", "click", None).expect("d");
        // El capture stopPropagation evita target Y bubble.
        assert_eq!(r.count, 1);
        assert_eq!(rt.stdout(), "CAP\n");
    }

    #[test]
    fn capture_true_shorthand_funciona() {
        // addEventListener(type, fn, true) — sin objeto options.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').addEventListener('click', \
                function(){ console.log('cap') }, true);",
        )
        .expect("e");
        rt.dispatch_event("btn", "click", None).expect("d");
        assert_eq!(rt.stdout(), "cap\n");
    }

    #[test]
    fn remove_event_listener_distingue_capture_de_bubble() {
        // Registrar el MISMO fn en capture Y bubble. removeEventListener
        // sin options sólo borra el bubble; el capture sigue activo.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "var f = function(){ console.log('x') }; \
             var o = document.getElementById('outer'); \
             o.addEventListener('click', f, true); \
             o.addEventListener('click', f, false); \
             o.removeEventListener('click', f); \
             document.getElementById('btn').onclick = function(){ console.log('b') };",
        )
        .expect("e");
        rt.dispatch_event("btn", "click", None).expect("d");
        // El capture sigue corriendo (no se removió); el bubble fue
        // removido — orden: capture x, target b.
        assert_eq!(rt.stdout(), "x\nb\n");
    }

    // ============= Fase 7.11 — el.dataset =============

    #[test]
    fn dataset_initial_se_lee_camelcase() {
        // data-foo-bar="hola" → el.dataset.fooBar === "hola"
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_dataset("x", "div", &[("foo-bar", "hola")])])
            .expect("e");
        let v = rt.eval("document.getElementById('x').dataset.fooBar").expect("e");
        assert_eq!(v, JsValue::String("hola".into()));
    }

    #[test]
    fn dataset_setter_publica_mutacion_con_kebab_key() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        assert!(rt.drain_dom_mutations().is_empty());
        rt.eval("document.getElementById('x').dataset.fooBar = 'nuevo'")
            .expect("set");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].id, "x");
        // El kind incluye el key en kebab (foo-bar), no en camelCase.
        assert_eq!(muts[0].kind, "dataset:foo-bar");
        assert_eq!(muts[0].value, "nuevo");
    }

    #[test]
    fn dataset_set_simple_se_lee_back() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval("document.getElementById('x').dataset.role = 'banner'")
            .expect("e");
        let v = rt.eval("document.getElementById('x').dataset.role").expect("e");
        assert_eq!(v, JsValue::String("banner".into()));
    }

    #[test]
    fn dataset_delete_publica_mutacion_remove() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_dataset("x", "div", &[("role", "main")])])
            .expect("e");
        rt.drain_dom_mutations();
        rt.eval("delete document.getElementById('x').dataset.role")
            .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "dataset-remove:role");
        // Y el getter después del delete devuelve undefined.
        let v = rt.eval("document.getElementById('x').dataset.role").expect("e");
        assert_eq!(v, JsValue::Undefined);
    }

    // ============= Fase 7.13 — options.once =============

    #[test]
    fn once_listener_se_dispara_una_sola_vez() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "")]).expect("e");
        rt.eval(
            "document.getElementById('btn').addEventListener('click', \
                function(){ console.log('hit') }, { once: true });",
        )
        .expect("e");
        rt.dispatch_event("btn", "click", None).expect("d1");
        rt.dispatch_event("btn", "click", None).expect("d2");
        rt.dispatch_event("btn", "click", None).expect("d3");
        // Sólo el primer dispatch corrió el handler.
        assert_eq!(rt.stdout(), "hit\n");
    }

    #[test]
    fn once_listener_no_afecta_otros_listeners() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "")]).expect("e");
        rt.eval(
            "var el = document.getElementById('btn'); \
             el.addEventListener('click', function(){ console.log('once') }, { once: true }); \
             el.addEventListener('click', function(){ console.log('forever') });",
        )
        .expect("e");
        rt.dispatch_event("btn", "click", None).expect("d1");
        rt.dispatch_event("btn", "click", None).expect("d2");
        // Primer dispatch: ambos. Segundo: sólo 'forever' (once se borró).
        assert_eq!(rt.stdout(), "once\nforever\nforever\n");
    }

    #[test]
    fn children_lista_hijos_con_parent_id_matching() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("a", "li", "p"),
            snap_with_parent("b", "li", "p"),
            snap_with_parent("c", "li", "p"),
            snap("other", "div", ""), // sin parent_id = p, no debe aparecer
        ])
        .expect("e");
        let v = rt
            .eval("document.getElementById('p').children.length")
            .expect("e");
        assert_eq!(v, JsValue::Number(3.0));
        let v = rt
            .eval("document.getElementById('p').children[0].id")
            .expect("e");
        assert_eq!(v, JsValue::String("a".into()));
    }

    #[test]
    fn first_last_element_child_funcionan() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("a", "li", "p"),
            snap_with_parent("b", "li", "p"),
        ])
        .expect("e");
        let v = rt
            .eval("document.getElementById('p').firstElementChild.id")
            .expect("e");
        assert_eq!(v, JsValue::String("a".into()));
        let v = rt
            .eval("document.getElementById('p').lastElementChild.id")
            .expect("e");
        assert_eq!(v, JsValue::String("b".into()));
    }

    #[test]
    fn children_vacios_es_array_length_0() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "div", "")]).expect("e");
        let v = rt
            .eval("document.getElementById('p').children.length")
            .expect("e");
        assert_eq!(v, JsValue::Number(0.0));
        let v = rt
            .eval("document.getElementById('p').firstElementChild")
            .expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn el_click_dispara_handler_programaticamente() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "")]).expect("e");
        rt.eval(
            "document.getElementById('btn').onclick = function(){ console.log('clicked') }; \
             document.getElementById('btn').click();",
        )
        .expect("e");
        assert_eq!(rt.stdout(), "clicked\n");
    }

    #[test]
    fn el_click_bubblea_por_ancestros() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("outer", "div", ""),
            snap_with_parent("btn", "button", "outer"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('outer').onclick = function(){ console.log('OUT') }; \
             document.getElementById('btn').onclick = function(){ console.log('BTN') }; \
             document.getElementById('btn').click();",
        )
        .expect("e");
        // click() reusa el dispatch normal: bubblea normalmente.
        assert_eq!(rt.stdout(), "BTN\nOUT\n");
    }

    #[test]
    fn el_focus_blur_disparan_eventos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("i", "input", "")]).expect("e");
        rt.eval(
            "var el = document.getElementById('i'); \
             el.onfocus = function(){ console.log('F') }; \
             el.onblur = function(){ console.log('B') }; \
             el.focus(); el.blur();",
        )
        .expect("e");
        assert_eq!(rt.stdout(), "F\nB\n");
    }

    #[test]
    fn children_refleja_createElement_appendChild() {
        // Después de appendChild, el child queda con _parent_id = parent.id
        // y debe aparecer en parent.children.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "ul", "")]).expect("e");
        rt.eval(
            "var li = document.createElement('li'); \
             li.id = 'fresh'; \
             document.getElementById('p').appendChild(li);",
        )
        .expect("e");
        let v = rt
            .eval("document.getElementById('p').children.length")
            .expect("e");
        assert_eq!(v, JsValue::Number(1.0));
        let v = rt
            .eval("document.getElementById('p').children[0].id")
            .expect("e");
        assert_eq!(v, JsValue::String("fresh".into()));
    }

