//! Tests de fetch/Response, bodyUsed, requestIdleCallback, PageVisibility, Observers, window events, XHR.
    use super::*;

    // ============= Fase 7.31 — fetch() async + Response =============

    #[test]
    fn fetch_devuelve_promise_y_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("var p = fetch('/api/x')").expect("e");
        // Devuelve un Promise.
        let v = rt.eval("p instanceof Promise").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "fetch");
        assert_eq!(muts[0].id, "__window__");
        // Payload tiene id=1, method=GET, url=/api/x, has_body=0, body="".
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[0], "1");
        assert_eq!(parts[1], "GET");
        assert_eq!(parts[2], "/api/x");
        assert_eq!(parts[3], "0");
        assert_eq!(parts[4], "");
    }

    #[test]
    fn resolve_fetch_dispara_then_con_response_ok() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var done = false; var capturedStatus = null; var capturedText = null; \
             fetch('/x').then(function(r) { \
                capturedStatus = r.status; \
                return r.text(); \
             }).then(function(t) { capturedText = t; done = true; });",
        )
        .expect("e");
        // Simular respuesta del chrome.
        rt.resolve_fetch(1, 200, "OK", "hola mundo", &[]).expect("resolve");
        let v = rt.eval("done").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("capturedStatus").expect("e");
        assert_eq!(v, JsValue::Number(200.0));
        let v = rt.eval("capturedText").expect("e");
        assert_eq!(v, JsValue::String("hola mundo".into()));
    }

    #[test]
    fn response_json_parsea_body_como_json() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var captured = null; \
             fetch('/x').then(function(r) { return r.json(); }).then(function(j) { captured = j; });",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", r#"{"name":"sergio","n":42}"#, &[])
            .expect("resolve");
        let v = rt.eval("captured.name").expect("e");
        assert_eq!(v, JsValue::String("sergio".into()));
        let v = rt.eval("captured.n").expect("e");
        assert_eq!(v, JsValue::Number(42.0));
    }

    #[test]
    fn response_ok_es_false_para_status_no_2xx() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var status = null; var ok = null; fetch('/x').then(function(r) { status = r.status; ok = r.ok; })").expect("e");
        rt.resolve_fetch(1, 404, "Not Found", "", &[]).expect("resolve");
        let v = rt.eval("status").expect("e");
        assert_eq!(v, JsValue::Number(404.0));
        let v = rt.eval("ok").expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

    #[test]
    fn reject_fetch_dispara_catch() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var err = null; fetch('/x').catch(function(e) { err = e.message; })")
            .expect("e");
        rt.reject_fetch(1, "network down").expect("reject");
        let v = rt.eval("err").expect("e");
        assert_eq!(v, JsValue::String("network down".into()));
    }

    #[test]
    fn fetch_post_publica_method_y_body() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('/api', {method: 'POST', body: 'hola'})").expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "POST");
        assert_eq!(parts[3], "1");
        assert_eq!(parts[4], "hola");
    }

    #[test]
    fn fetch_con_headers_objeto() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "fetch('/api', {headers: {'X-Token': 'abc', 'Content-Type': 'text/plain'}})",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        // Headers van a partir del índice 5 en pares.
        let mut hdr_map = std::collections::HashMap::new();
        let mut i = 5;
        while i + 1 < parts.len() {
            hdr_map.insert(parts[i].to_string(), parts[i + 1].to_string());
            i += 2;
        }
        assert_eq!(hdr_map.get("X-Token").map(|s| s.as_str()), Some("abc"));
        assert_eq!(hdr_map.get("Content-Type").map(|s| s.as_str()), Some("text/plain"));
    }

    #[test]
    fn fetch_con_headers_class() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var h = new Headers(); h.set('Authorization', 'Bearer 123'); \
             fetch('/api', {headers: h})",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        // Headers class lowercases name al guardar.
        assert!(parts.iter().any(|p| *p == "authorization"));
        assert!(parts.iter().any(|p| *p == "Bearer 123"));
    }

    #[test]
    fn response_headers_get_devuelve_value() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var ct = null; fetch('/x').then(function(r) { ct = r.headers.get('content-type'); })").expect("e");
        let headers = vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("x-foo".to_string(), "bar".to_string()),
        ];
        rt.resolve_fetch(1, 200, "OK", "{}", &headers).expect("r");
        let v = rt.eval("ct").expect("e");
        assert_eq!(v, JsValue::String("application/json".into()));
    }

    #[test]
    fn abort_controller_signal_aborted_inicialmente_false() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var c = new AbortController()").expect("e");
        let v = rt.eval("c.signal.aborted").expect("e");
        assert_eq!(v, JsValue::Bool(false));
        rt.eval("c.abort()").expect("e");
        let v = rt.eval("c.signal.aborted").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn abort_controller_abort_rechaza_fetch_pendiente() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var c = new AbortController(); var err = null; \
             fetch('/x', {signal: c.signal}).catch(function(e) { err = e.message; }); \
             c.abort();",
        )
        .expect("e");
        let v = rt.eval("err").expect("e");
        // El mensaje incluye 'AbortError'.
        if let JsValue::String(s) = v {
            assert!(s.contains("AbortError"), "msg: {s}");
        } else {
            panic!("expected string, got {v:?}");
        }
    }

    #[test]
    fn abort_signal_ya_aborted_rechaza_fetch_inmediato() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var c = new AbortController(); c.abort(); \
             var err = null; \
             fetch('/x', {signal: c.signal}).catch(function(e) { err = e.message; });",
        )
        .expect("e");
        let v = rt.eval("err").expect("e");
        if let JsValue::String(s) = v {
            assert!(s.contains("AbortError"), "msg: {s}");
        } else {
            panic!("expected string");
        }
    }

    #[test]
    fn headers_class_api_basica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var h = new Headers({'X-Foo': 'bar'}); h.append('x-foo', 'baz'); \
             h.set('Other', '1');",
        )
        .expect("e");
        // get case-insensitive + multiple values joined.
        let v = rt.eval("h.get('X-Foo')").expect("e");
        assert_eq!(v, JsValue::String("bar, baz".into()));
        let v = rt.eval("h.has('Other')").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("h.has('Missing')").expect("e");
        assert_eq!(v, JsValue::Bool(false));
        // delete.
        rt.eval("h.delete('Other')").expect("e");
        let v = rt.eval("h.has('Other')").expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

    // ============= Fase 7.35 — bodyUsed enforcement =============

    #[test]
    fn body_used_pasa_a_true_tras_text() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var used = null; fetch('/x').then(function(r) { r.text(); used = r.bodyUsed; })")
            .expect("e");
        rt.resolve_fetch(1, 200, "OK", "hola", &[]).expect("r");
        let v = rt.eval("used").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn body_used_segunda_lectura_rechaza_con_type_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var err = null; \
             fetch('/x').then(function(r) { r.text(); return r.text(); }) \
                        .catch(function(e) { err = e.message; });",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "hola", &[]).expect("r");
        let v = rt.eval("err").expect("e");
        if let JsValue::String(s) = v {
            assert!(s.contains("already read"), "msg: {s}");
        } else {
            panic!("expected string, got {v:?}");
        }
    }

    // ============= Fase 7.43 — requestIdleCallback =============

    #[test]
    fn request_idle_callback_corre_callback_con_deadline() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var ran = false; var deadline = null; \
             requestIdleCallback(function(d) { ran = true; deadline = d; });",
        )
        .expect("e");
        // setTimeout(0) → fire en tick(0).
        rt.tick(0).expect("tick");
        let v = rt.eval("ran").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        // El deadline tiene didTimeout=false (sin opts.timeout) y timeRemaining() > 0.
        let v = rt.eval("deadline.didTimeout").expect("e");
        assert_eq!(v, JsValue::Bool(false));
        let v = rt.eval("deadline.timeRemaining()").expect("e");
        assert_eq!(v, JsValue::Number(50.0));
    }

    #[test]
    fn request_idle_callback_con_timeout_marca_did_timeout() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var got = null; \
             requestIdleCallback(function(d) { got = d.didTimeout; }, {timeout: 30});",
        )
        .expect("e");
        // El delay queda en min(30, 50) = 30ms — tick(30) lo fire.
        rt.tick(30).expect("tick");
        let v = rt.eval("got").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn cancel_idle_callback_evita_el_disparo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var ran = false; \
             var id = requestIdleCallback(function() { ran = true; }); \
             cancelIdleCallback(id);",
        )
        .expect("e");
        rt.tick(0).expect("tick");
        let v = rt.eval("ran").expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

    // ============= Fase 7.42 — Page Visibility =============

    #[test]
    fn visibility_inicial_es_visible() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        assert_eq!(rt.eval("document.hidden").expect("e"), JsValue::Bool(false));
        assert_eq!(
            rt.eval("document.visibilityState").expect("e"),
            JsValue::String("visible".into())
        );
    }

    #[test]
    fn set_visibility_true_actualiza_hidden_y_state() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_visibility(true).expect("hide");
        assert_eq!(rt.eval("document.hidden").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("document.visibilityState").expect("e"),
            JsValue::String("hidden".into())
        );
    }

    #[test]
    fn set_visibility_dispara_visibilitychange() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var states = []; \
             window.addEventListener('visibilitychange', function() { \
                states.push(document.visibilityState); \
             });",
        )
        .expect("e");
        rt.set_visibility(true).expect("hide");
        rt.set_visibility(false).expect("show");
        let v = rt.eval("states.join(',')").expect("e");
        assert_eq!(v, JsValue::String("hidden,visible".into()));
    }

    #[test]
    fn set_visibility_idempotente_no_dispara_cuando_no_cambia() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var n = 0; \
             window.addEventListener('visibilitychange', function() { n++; });",
        )
        .expect("e");
        // Ya está visible: setear visible de nuevo no debe disparar.
        rt.set_visibility(false).expect("show");
        rt.set_visibility(false).expect("show");
        let v = rt.eval("n").expect("e");
        assert_eq!(v, JsValue::Number(0.0));
        rt.set_visibility(true).expect("hide");
        rt.set_visibility(true).expect("hide");
        let v = rt.eval("n").expect("e");
        assert_eq!(v, JsValue::Number(1.0));
    }

    // ============= Fase 7.40 — Observers stub =============

    #[test]
    fn mutation_observer_existe_y_no_tira_al_construir() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var mo = new MutationObserver(function(records) { /* no */ });")
            .expect("e");
        let v = rt.eval("mo instanceof MutationObserver").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn mutation_observer_observe_y_take_records_no_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var mo = new MutationObserver(function() {}); \
             mo.observe(document.body, {childList: true, subtree: true}); \
             var recs = mo.takeRecords();",
        )
        .expect("e");
        let v = rt.eval("Array.isArray(recs)").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("recs.length").expect("e");
        assert_eq!(v, JsValue::Number(0.0));
    }

    #[test]
    fn intersection_observer_expone_root_y_thresholds() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var io = new IntersectionObserver(function() {}, \
                {rootMargin: '10px', threshold: [0, 0.5, 1.0]});",
        )
        .expect("e");
        let v = rt.eval("io.rootMargin").expect("e");
        assert_eq!(v, JsValue::String("10px".into()));
        let v = rt.eval("io.thresholds.length").expect("e");
        assert_eq!(v, JsValue::Number(3.0));
        let v = rt.eval("io.thresholds[1]").expect("e");
        assert_eq!(v, JsValue::Number(0.5));
    }

    #[test]
    fn intersection_observer_threshold_escalar_se_envuelve_en_array() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var io = new IntersectionObserver(function() {}, {threshold: 0.25});")
            .expect("e");
        let v = rt.eval("io.thresholds.length").expect("e");
        assert_eq!(v, JsValue::Number(1.0));
        let v = rt.eval("io.thresholds[0]").expect("e");
        assert_eq!(v, JsValue::Number(0.25));
    }

    #[test]
    fn resize_observer_observe_y_disconnect_no_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var ro = new ResizeObserver(function() {}); \
             ro.observe(document.body); \
             ro.disconnect();",
        )
        .expect("e");
    }

    // ============= Fase 7.39 — window events =============

    #[test]
    fn document_add_event_listener_domcontentloaded_corre() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var ready = false; \
             document.addEventListener('DOMContentLoaded', function() { ready = true; });",
        )
        .expect("e");
        let r = rt.dispatch_document_event("DOMContentLoaded", None, None).expect("d");
        assert_eq!(r.count, 1);
        assert_eq!(rt.eval("ready").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn document_on_property_y_listener_corren_juntos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var n = 0; \
             document.onclick = function() { n++; }; \
             document.addEventListener('click', function() { n++; });",
        )
        .expect("e");
        let r = rt.dispatch_document_event("click", None, None).expect("d");
        assert_eq!(r.count, 2);
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn document_remove_event_listener_cancela() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var n = 0; var h = function() { n++; }; \
             document.addEventListener('click', h); \
             document.removeEventListener('click', h);",
        )
        .expect("e");
        let r = rt.dispatch_document_event("click", None, None).expect("d");
        assert_eq!(r.count, 0);
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn document_listener_once_se_dispara_una_sola_vez() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var n = 0; \
             document.addEventListener('foo', function() { n++; }, { once: true });",
        )
        .expect("e");
        rt.dispatch_document_event("foo", None, None).expect("d");
        rt.dispatch_document_event("foo", None, None).expect("d");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn document_click_delegacion_trae_target_y_currenttarget() {
        // Modelo de delegación: el evento bubbleó desde #btn; event.target es
        // el botón, event.currentTarget es el document.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.set_elements(&[snap("btn", "button", "Click")]).expect("els");
        rt.eval(
            "var tgt = null; var cur = null; \
             document.addEventListener('click', function(e) { tgt = e.target.id; cur = (e.currentTarget === document); });",
        )
        .expect("e");
        let r = rt
            .dispatch_document_event("click", None, Some("btn"))
            .expect("d");
        assert_eq!(r.count, 1);
        assert_eq!(rt.eval("tgt").expect("e"), JsValue::String("btn".into()));
        assert_eq!(rt.eval("cur").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn document_prevent_default_se_refleja_en_result() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "document.addEventListener('click', function(e) { e.preventDefault(); });",
        )
        .expect("e");
        let r = rt.dispatch_document_event("click", None, None).expect("d");
        assert!(r.default_prevented);
    }

    #[test]
    fn window_add_event_listener_scroll_corre_handler() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var got = null; \
             window.addEventListener('scroll', function() { got = window.scrollY; });",
        )
        .expect("e");
        rt.set_scroll(0.0, 123.0).expect("scroll");
        let r = rt.dispatch_window_event("scroll", None).expect("d");
        assert_eq!(r.count, 1);
        let v = rt.eval("got").expect("e");
        assert_eq!(v, JsValue::Number(123.0));
    }

    #[test]
    fn window_on_load_property_corre() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval("var loaded = false; window.onload = function() { loaded = true; };")
            .expect("e");
        rt.dispatch_window_event("load", None).expect("d");
        let v = rt.eval("loaded").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn window_event_listener_y_on_property_corren_juntos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var n = 0; \
             window.onresize = function() { n++; }; \
             window.addEventListener('resize', function() { n++; }); \
             window.addEventListener('resize', function() { n++; });",
        )
        .expect("e");
        let r = rt.dispatch_window_event("resize", None).expect("d");
        assert_eq!(r.count, 3);
    }

    #[test]
    fn window_remove_event_listener_lo_quita() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var n = 0; var f = function() { n++; }; \
             window.addEventListener('scroll', f); \
             window.removeEventListener('scroll', f);",
        )
        .expect("e");
        rt.dispatch_window_event("scroll", None).expect("d");
        let v = rt.eval("n").expect("e");
        assert_eq!(v, JsValue::Number(0.0));
    }

    #[test]
    fn window_add_event_listener_once_se_borra_tras_disparar() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var n = 0; \
             window.addEventListener('scroll', function() { n++; }, {once: true});",
        )
        .expect("e");
        rt.dispatch_window_event("scroll", None).expect("d");
        rt.dispatch_window_event("scroll", None).expect("d");
        rt.dispatch_window_event("scroll", None).expect("d");
        let v = rt.eval("n").expect("e");
        assert_eq!(v, JsValue::Number(1.0));
    }

    // ============= Fase 7.38 — XMLHttpRequest =============

    #[test]
    fn xhr_open_setea_ready_state_1() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval("var x = new XMLHttpRequest(); x.open('GET', '/api')")
            .expect("e");
        let v = rt.eval("x.readyState").expect("e");
        assert_eq!(v, JsValue::Number(1.0));
    }

    #[test]
    fn xhr_open_async_false_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        let res = rt.eval(
            "var err = null; \
             try { var x = new XMLHttpRequest(); x.open('GET', '/api', false); } \
             catch (e) { err = e.message; } \
             err",
        )
        .expect("e");
        if let JsValue::String(s) = res {
            assert!(s.contains("no soportado"), "msg: {s}");
        } else {
            panic!("expected string, got {res:?}");
        }
    }

    #[test]
    fn xhr_send_publica_mutacion_fetch() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var x = new XMLHttpRequest(); \
             x.open('POST', '/api/x'); \
             x.setRequestHeader('X-Token', 'abc'); \
             x.send('hola');",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "fetch");
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "POST");
        assert_eq!(parts[2], "https://example.com/api/x");
        assert_eq!(parts[3], "1");
        assert_eq!(parts[4], "hola");
        assert!(parts.iter().any(|p| *p == "X-Token"));
        assert!(parts.iter().any(|p| *p == "abc"));
    }

    #[test]
    fn xhr_send_dispara_ready_state_2() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var states = []; var x = new XMLHttpRequest(); \
             x.onreadystatechange = function() { states.push(x.readyState); }; \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        // Por open: 1, por send: 2.
        let v = rt.eval("states.join(',')").expect("e");
        assert_eq!(v, JsValue::String("1,2".into()));
    }

    #[test]
    fn xhr_resolve_fetch_dispara_onload_y_response_text() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var loaded = false; var txt = null; var s = null; \
             var x = new XMLHttpRequest(); \
             x.onload = function() { loaded = true; txt = x.responseText; s = x.status; }; \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        // El id es 1 (primer fetch del runtime).
        rt.resolve_fetch(1, 200, "OK", "hola mundo", &[]).expect("r");
        let v = rt.eval("loaded").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("txt").expect("e");
        assert_eq!(v, JsValue::String("hola mundo".into()));
        let v = rt.eval("s").expect("e");
        assert_eq!(v, JsValue::Number(200.0));
        let v = rt.eval("x.readyState").expect("e");
        assert_eq!(v, JsValue::Number(4.0));
    }

    #[test]
    fn xhr_get_response_header_case_insensitive() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var x = new XMLHttpRequest(); x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        let headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        rt.resolve_fetch(1, 200, "OK", "{}", &headers).expect("r");
        let v = rt.eval("x.getResponseHeader('content-type')").expect("e");
        assert_eq!(v, JsValue::String("application/json".into()));
        let v = rt.eval("x.getResponseHeader('Content-Type')").expect("e");
        assert_eq!(v, JsValue::String("application/json".into()));
        let v = rt.eval("x.getResponseHeader('missing')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    // ============= Fase 7.47 — XHR responseType + Blob =============

    #[test]
    fn xhr_response_type_json_parsea_el_body() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var r = null; var x = new XMLHttpRequest(); x.responseType = 'json'; \
             x.onload = function() { r = x.response; }; x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", r#"{"name":"sergio","n":7}"#, &[]).expect("r");
        assert_eq!(rt.eval("r.name").expect("e"), JsValue::String("sergio".into()));
        assert_eq!(rt.eval("r.n").expect("e"), JsValue::Number(7.0));
    }

    #[test]
    fn xhr_response_type_json_invalido_da_null() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var r = 'x'; var x = new XMLHttpRequest(); x.responseType = 'json'; \
             x.onload = function() { r = x.response; }; x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "{roto", &[]).expect("r");
        assert_eq!(rt.eval("r").expect("e"), JsValue::Null);
    }

    #[test]
    fn xhr_response_type_arraybuffer_da_bytes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var b0 = null; var b1 = null; var isBuf = null; \
             var x = new XMLHttpRequest(); x.responseType = 'arraybuffer'; \
             x.onload = function() { isBuf = x.response instanceof ArrayBuffer; \
                var v = new Uint8Array(x.response); b0 = v[0]; b1 = v[1]; }; \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "AB", &[]).expect("r");
        assert_eq!(rt.eval("isBuf").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("b0").expect("e"), JsValue::Number(65.0));
        assert_eq!(rt.eval("b1").expect("e"), JsValue::Number(66.0));
    }

    #[test]
    fn xhr_response_type_blob_da_blob_con_type() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var size = null; var type = null; var txt = null; \
             var x = new XMLHttpRequest(); x.responseType = 'blob'; \
             x.onload = function() { size = x.response.size; type = x.response.type; \
                x.response.text().then(function(t) { txt = t; }); }; \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        let headers = vec![("Content-Type".to_string(), "text/plain".to_string())];
        rt.resolve_fetch(1, 200, "OK", "hola", &headers).expect("r");
        assert_eq!(rt.eval("size").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("type").expect("e"), JsValue::String("text/plain".into()));
        assert_eq!(rt.eval("txt").expect("e"), JsValue::String("hola".into()));
    }

    #[test]
    fn xhr_response_type_text_default_es_string() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var r = null; var x = new XMLHttpRequest(); \
             x.onload = function() { r = x.response; }; x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "plano", &[]).expect("r");
        assert_eq!(rt.eval("r").expect("e"), JsValue::String("plano".into()));
    }

    #[test]
    fn blob_constructor_y_slice() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var b = new Blob(['abc', 'def'], { type: 'text/plain' }); \
             var sz = b.size; var ty = b.type; \
             var sl = b.slice(1, 4); var slTxt = null; \
             sl.text().then(function(t) { slTxt = t; });",
        )
        .expect("e");
        assert_eq!(rt.eval("sz").expect("e"), JsValue::Number(6.0));
        assert_eq!(rt.eval("ty").expect("e"), JsValue::String("text/plain".into()));
        assert_eq!(rt.eval("slTxt").expect("e"), JsValue::String("bcd".into()));
    }

    #[test]
    fn xhr_reject_fetch_dispara_onerror() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var errored = false; var x = new XMLHttpRequest(); \
             x.onerror = function() { errored = true; }; \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.reject_fetch(1, "network down").expect("r");
        let v = rt.eval("errored").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("x.readyState").expect("e");
        assert_eq!(v, JsValue::Number(4.0));
        let v = rt.eval("x.status").expect("e");
        assert_eq!(v, JsValue::Number(0.0));
    }

    #[test]
    fn xhr_abort_dispara_onabort_y_descarta_resolve_posterior() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var aborted = false; var loaded = false; var x = new XMLHttpRequest(); \
             x.onabort = function() { aborted = true; }; \
             x.onload = function() { loaded = true; }; \
             x.open('GET', '/x'); x.send(); x.abort();",
        )
        .expect("e");
        // El abort eliminó al XHR del pending — el resolve posterior debe
        // ser no-op para el XHR (no encuentra entrada y cae al Promise pending,
        // que tampoco existe).
        rt.resolve_fetch(1, 200, "OK", "hola", &[]).expect("r");
        let v = rt.eval("aborted").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("loaded").expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

    // ============= Fase 7.48 — XHR eventos de progreso + addEventListener =============

    #[test]
    fn xhr_addeventlistener_load_dispara() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var hits = 0; var x = new XMLHttpRequest(); \
             x.addEventListener('load', function() { hits++; }); \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "hola", &[]).expect("r");
        assert_eq!(rt.eval("hits").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn xhr_dispara_loadstart_progress_load_loadend_en_orden() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var seq = []; var x = new XMLHttpRequest(); \
             ['loadstart','progress','load','loadend'].forEach(function(t) { \
                 x.addEventListener(t, function() { seq.push(t); }); }); \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        // loadstart se dispara en send().
        assert_eq!(rt.eval("seq.join(',')").expect("e"), JsValue::String("loadstart".into()));
        rt.resolve_fetch(1, 200, "OK", "hola", &[]).expect("r");
        assert_eq!(
            rt.eval("seq.join(',')").expect("e"),
            JsValue::String("loadstart,progress,load,loadend".into())
        );
    }

    #[test]
    fn xhr_progress_event_reporta_loaded_y_total() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var lc = null; var ld = null; var tot = null; \
             var x = new XMLHttpRequest(); \
             x.onprogress = function(e) { lc = e.lengthComputable; ld = e.loaded; tot = e.total; }; \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "hola", &[]).expect("r");
        assert_eq!(rt.eval("lc").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ld").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("tot").expect("e"), JsValue::Number(4.0));
    }

    #[test]
    fn xhr_error_dispara_error_y_loadend() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var seq = []; var x = new XMLHttpRequest(); \
             x.addEventListener('error', function() { seq.push('error'); }); \
             x.addEventListener('loadend', function() { seq.push('loadend'); }); \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.reject_fetch(1, "boom").expect("r");
        assert_eq!(rt.eval("seq.join(',')").expect("e"), JsValue::String("error,loadend".into()));
    }

    #[test]
    fn xhr_remove_event_listener_silencia() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var hits = 0; var f = function() { hits++; }; \
             var x = new XMLHttpRequest(); \
             x.addEventListener('load', f); x.removeEventListener('load', f); \
             x.open('GET', '/x'); x.send();",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "hola", &[]).expect("r");
        assert_eq!(rt.eval("hits").expect("e"), JsValue::Number(0.0));
    }

    // ============= Fase 7.49 — Blob.stream() =============

    #[test]
    fn blob_stream_emite_los_bytes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var bytes = null; var done2 = null; \
             var b = new Blob(['Hi']); \
             var rd = b.stream().getReader(); \
             rd.read().then(function(r) { \
                 bytes = [r.value[0], r.value[1]]; \
                 return rd.read(); \
             }).then(function(r2) { done2 = r2.done; });",
        )
        .expect("e");
        assert_eq!(rt.eval("bytes[0]").expect("e"), JsValue::Number(72.0)); // 'H'
        assert_eq!(rt.eval("bytes[1]").expect("e"), JsValue::Number(105.0)); // 'i'
        assert_eq!(rt.eval("done2").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.50 — URL.createObjectURL / revokeObjectURL =============

    #[test]
    fn url_create_object_url_resuelve_al_blob() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var b = new Blob(['x'], { type: 'text/plain' }); \
             var u = URL.createObjectURL(b); \
             var isBlobScheme = u.indexOf('blob:') === 0; \
             var resolved = globalThis.__puriy_resolve_blob_url(u); \
             var same = resolved === b;",
        )
        .expect("e");
        assert_eq!(rt.eval("isBlobScheme").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("same").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn url_revoke_object_url_borra_la_entrada() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var b = new Blob(['x']); var u = URL.createObjectURL(b); \
             URL.revokeObjectURL(u); \
             var resolved = globalThis.__puriy_resolve_blob_url(u);",
        )
        .expect("e");
        assert_eq!(rt.eval("resolved").expect("e"), JsValue::Null);
    }

    #[test]
    fn url_create_object_url_da_urls_unicas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var u1 = URL.createObjectURL(new Blob(['a'])); \
             var u2 = URL.createObjectURL(new Blob(['b'])); \
             var distintas = u1 !== u2;",
        )
        .expect("e");
        assert_eq!(rt.eval("distintas").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.51 — URLSearchParams =============

    #[test]
    fn usp_parsea_string_y_get() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new URLSearchParams('?a=1&b=hola+mundo&a=2');").expect("e");
        assert_eq!(rt.eval("p.get('a')").expect("e"), JsValue::String("1".into()));
        assert_eq!(rt.eval("p.get('b')").expect("e"), JsValue::String("hola mundo".into()));
        assert_eq!(rt.eval("p.getAll('a').join(',')").expect("e"), JsValue::String("1,2".into()));
        assert_eq!(rt.eval("p.has('b')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("p.has('z')").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn usp_set_reemplaza_y_append_agrega() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var p = new URLSearchParams('a=1&a=2&b=3'); \
             p.set('a', '9'); p.append('c', '4');",
        )
        .expect("e");
        // set deja una sola 'a'.
        assert_eq!(rt.eval("p.getAll('a').join(',')").expect("e"), JsValue::String("9".into()));
        assert_eq!(rt.eval("p.get('c')").expect("e"), JsValue::String("4".into()));
    }

    #[test]
    fn usp_tostring_encoda_form_urlencoded() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new URLSearchParams(); p.append('q', 'a b&c'); p.append('x', 'ñ');").expect("e");
        // espacio → '+', '&' → %26, 'ñ' → %C3%B1 (UTF-8).
        assert_eq!(
            rt.eval("p.toString()").expect("e"),
            JsValue::String("q=a+b%26c&x=%C3%B1".into())
        );
    }

    #[test]
    fn usp_construye_desde_objeto_y_itera() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var p = new URLSearchParams({ a: '1', b: '2' }); \
             var seq = []; for (var pair of p) { seq.push(pair[0] + '=' + pair[1]); }",
        )
        .expect("e");
        assert_eq!(rt.eval("seq.join('&')").expect("e"), JsValue::String("a=1&b=2".into()));
    }

    #[test]
    fn usp_como_body_de_fetch_se_serializa() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('/api', { method: 'POST', body: new URLSearchParams({ k: 'v w' }) });").expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        // [3] has_body, [4] body string.
        assert_eq!(parts[4], "k=v+w");
    }

    // ============= Fase 7.52 — TextEncoder / TextDecoder =============

    #[test]
    fn textencoder_encode_utf8_ascii_y_multibyte() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var e = new TextEncoder(); var a = e.encode('Añ'); \
             var bytes = []; for (var i=0;i<a.length;i++) bytes.push(a[i]);",
        )
        .expect("e");
        // 'A' = 0x41, 'ñ' = 0xC3 0xB1.
        assert_eq!(rt.eval("bytes.join(',')").expect("e"), JsValue::String("65,195,177".into()));
    }

    #[test]
    fn textencoder_encode_emoji_surrogate_pair() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var a = new TextEncoder().encode('😀'); \
             var bytes = []; for (var i=0;i<a.length;i++) bytes.push(a[i]);",
        )
        .expect("e");
        // U+1F600 → F0 9F 98 80.
        assert_eq!(rt.eval("bytes.join(',')").expect("e"), JsValue::String("240,159,152,128".into()));
    }

    #[test]
    fn textdecoder_decode_round_trip() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var s = 'Hola ñ 😀 mundo'; \
             var bytes = new TextEncoder().encode(s); \
             var back = new TextDecoder().decode(bytes); \
             var ok = back === s;",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn textdecoder_decode_desde_arraybuffer() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var buf = new ArrayBuffer(3); var v = new Uint8Array(buf); \
             v[0]=72; v[1]=105; v[2]=33; \
             var s = new TextDecoder().decode(buf);",
        )
        .expect("e");
        assert_eq!(rt.eval("s").expect("e"), JsValue::String("Hi!".into()));
    }

    // ============= Fase 7.53 — btoa / atob =============

    #[test]
    fn btoa_codifica_base64() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var a = btoa('Hello'); var b = btoa('M'); var c = btoa('Ma');").expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::String("SGVsbG8=".into()));
        assert_eq!(rt.eval("b").expect("e"), JsValue::String("TQ==".into()));
        assert_eq!(rt.eval("c").expect("e"), JsValue::String("TWE=".into()));
    }

    #[test]
    fn atob_decodifica_base64() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var a = atob('SGVsbG8='); var b = atob('TQ==');").expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::String("Hello".into()));
        assert_eq!(rt.eval("b").expect("e"), JsValue::String("M".into()));
    }

    #[test]
    fn btoa_atob_round_trip_y_btoa_rechaza_no_latin1() {
        let mut rt = JsRuntime::new().expect("rt");
        // Construimos el binary string en runtime (bytes 0 y 255 incluidos)
        // para no embeber un NUL literal en el source.
        rt.eval(
            "var s = String.fromCharCode(98,105,0,255,33); \
             var ok = atob(btoa(s)) === s;",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        // '€' = U+20AC (8364) está fuera de Latin1 → btoa debe tirar.
        let threw = rt.eval(
            "var threw = false; try { btoa('€'); } catch (e) { threw = true; } threw;",
        )
        .expect("e");
        assert_eq!(threw, JsValue::Bool(true));
    }

    // ============= Fase 7.37 — URL relativa contra base =============

    #[test]
    fn fetch_url_absoluta_de_path_resuelve_contra_origin() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/page", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('/api/x')").expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[2], "https://example.com/api/x");
    }

    #[test]
    fn fetch_url_absoluta_completa_se_respeta() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/page", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('https://other.com/raw')").expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[2], "https://other.com/raw");
    }

    #[test]
    fn fetch_url_relativa_resuelve_contra_directorio_base() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/b/page.html", "b")
            .expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('foo.json')").expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[2], "https://example.com/a/b/foo.json");
    }

    #[test]
    fn fetch_url_protocol_relative_hereda_scheme() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/page", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('//cdn.example.com/lib.js')").expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[2], "https://cdn.example.com/lib.js");
    }

    #[test]
    fn fetch_url_solo_query_reemplaza_query_de_base() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/page", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('?q=hola')").expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[2], "https://example.com/page?q=hola");
    }

    // ============= Fase 7.46 — normalización de segmentos =============

    fn resolved_url(rt: &mut JsRuntime, rel: &str) -> String {
        rt.drain_dom_mutations();
        rt.eval(&format!("fetch({rel:?})")).expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<String> = muts[0].value.split('\u{001D}').map(|s| s.to_string()).collect();
        parts[2].clone()
    }

    #[test]
    fn url_relativa_colapsa_dotdot() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/b/page.html", "b").expect("d");
        assert_eq!(resolved_url(&mut rt, "../x.json"), "https://example.com/a/x.json");
        assert_eq!(resolved_url(&mut rt, "../../x.json"), "https://example.com/x.json");
    }

    #[test]
    fn url_relativa_colapsa_dot() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/b/page.html", "b").expect("d");
        assert_eq!(resolved_url(&mut rt, "./x.json"), "https://example.com/a/b/x.json");
        assert_eq!(resolved_url(&mut rt, "c/./d/../e"), "https://example.com/a/b/c/e");
    }

    #[test]
    fn url_absoluta_de_path_colapsa_segmentos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/b/page.html", "b").expect("d");
        assert_eq!(resolved_url(&mut rt, "/x/y/../z"), "https://example.com/x/z");
    }

    #[test]
    fn url_dotdot_no_escapa_la_raiz() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/page.html", "b").expect("d");
        // Más `..` que niveles → se clava en la raíz, no escapa el origin.
        assert_eq!(resolved_url(&mut rt, "../../../x"), "https://example.com/x");
    }

    #[test]
    fn url_dotdot_final_preserva_slash() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/b/page.html", "b").expect("d");
        // Último segmento `..` deja directorio con slash final (WHATWG).
        assert_eq!(resolved_url(&mut rt, "c/d/.."), "https://example.com/a/b/c/");
    }

    #[test]
    fn url_relativa_normaliza_pero_preserva_query() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/b/page.html", "b").expect("d");
        assert_eq!(
            resolved_url(&mut rt, "../api?id=1#frag"),
            "https://example.com/a/api?id=1#frag"
        );
    }

