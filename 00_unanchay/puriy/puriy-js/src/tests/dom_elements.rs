//! Tests del DOM: elementos, eventos, timers, bubbling, capture, dataset, sibling, createElement, cloneNode.
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

