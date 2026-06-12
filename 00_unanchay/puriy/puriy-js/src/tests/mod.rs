//! Módulo raíz de tests — helpers compartidos + smoke tests del runtime.
    pub use super::*;

    // DOM: eventos, timers, bubbling, capture, dataset, event.key, Element.value
    mod dom_events;
    // DOM: sibling, insertBefore, createElement, appendChild, cloneNode, tagName, etc.
    mod dom_tree;
    mod dom_style;
    mod fetch_xhr;
    mod streams_url;
    mod websocket_comms;
    // Navigator: cookie, cache, permisos, geolocation, clipboard, matchMedia, screen, serviceWorker...
    mod nav_basic;
    // Navigator: locks, mediaSession, vibration, gamepad, credentials, payment, serial, HID, USB...
    mod nav_device;
    // Hardware APIs de acceso físico y P2P: Fullscreen, BT, NFC, WebAuthn, WebTransport, Push...
    mod hw_io;
    // Hardware APIs de red y datos: Presentation, IndexedDB, WebRTC, Workers...
    mod hw_net;
    // Medios codificados: WebAudio, WebCodecs, MediaRecorder, MSE, EME, MediaCapabilities
    mod media_codecs;
    // Canvas, WebGL, GPU, XR y APIs de UI avanzadas
    mod canvas_gpu;
    mod lang_es2024;

    /// Helper para tests que crean un runtime, evalúan, y desempaquetan
    /// el JsValue. Pánico claro si algo falla.
    pub(super) fn eval(src: &str) -> JsValue {
        let mut rt = JsRuntime::new().expect("instanciar QuickJS");
        rt.eval(src).expect("eval no debe fallar")
    }

    #[test]
    fn runtime_arranca_sin_eval() {
        // Smoke test: ¿pasa wasm_validate + _initialize + qjs_init?
        let rt = JsRuntime::new().expect("instanciar runtime");
        assert!(rt.fuel_remaining() > 0);
    }

    #[test]
    fn eval_aritmetica_basica() {
        match eval("2 + 3") {
            JsValue::Number(n) => assert_eq!(n, 5.0),
            other => panic!("esperaba Number(5), obtuve {other:?}"),
        }
    }

    #[test]
    fn eval_string_literal() {
        match eval("'hola ' + 'mundo'") {
            JsValue::String(s) => assert_eq!(s, "hola mundo"),
            other => panic!("esperaba String, obtuve {other:?}"),
        }
    }

    #[test]
    fn eval_undefined_y_null() {
        assert_eq!(eval("undefined"), JsValue::Undefined);
        assert_eq!(eval("null"), JsValue::Null);
    }

    #[test]
    fn eval_booleanos() {
        assert_eq!(eval("true"), JsValue::Bool(true));
        assert_eq!(eval("false"), JsValue::Bool(false));
        assert_eq!(eval("1 === 1"), JsValue::Bool(true));
    }

    #[test]
    fn eval_floats() {
        match eval("3.14 * 2") {
            JsValue::Number(n) => assert!((n - 6.28).abs() < 1e-9),
            other => panic!("esperaba Number, obtuve {other:?}"),
        }
    }

    #[test]
    fn eval_estado_persiste_entre_llamadas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var x = 10").expect("decl no debe fallar");
        let v = rt.eval("x * 2").expect("segunda eval");
        assert_eq!(v, JsValue::Number(20.0));
    }

    #[test]
    fn eval_syntax_error_devuelve_runtime_error() {
        let mut rt = JsRuntime::new().expect("rt");
        let err = rt.eval("var !!! = 1").expect_err("sintaxis rota");
        match err {
            JsError::Runtime(msg) => {
                let l = msg.to_lowercase();
                assert!(
                    l.contains("syntax") || l.contains("expected") || l.contains("unexpected"),
                    "mensaje no parece SyntaxError: {msg}"
                );
            }
            other => panic!("esperaba Runtime, obtuve {other:?}"),
        }
    }

    #[test]
    fn eval_throw_explicito_devuelve_runtime_error() {
        let mut rt = JsRuntime::new().expect("rt");
        let err = rt.eval("throw new Error('boom')").expect_err("throw");
        match err {
            JsError::Runtime(msg) => assert!(msg.contains("boom")),
            other => panic!("esperaba Runtime, obtuve {other:?}"),
        }
    }

    #[test]
    fn eval_reference_error() {
        let mut rt = JsRuntime::new().expect("rt");
        let err = rt
            .eval("variable_que_no_existe_jamas")
            .expect_err("ref err");
        match err {
            JsError::Runtime(msg) => {
                let l = msg.to_lowercase();
                assert!(l.contains("not defined") || l.contains("reference"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn jsvalue_to_display_cubre_los_casos_basicos() {
        assert_eq!(JsValue::Undefined.to_display_string(), "undefined");
        assert_eq!(JsValue::Null.to_display_string(), "null");
        assert_eq!(JsValue::Bool(true).to_display_string(), "true");
        assert_eq!(JsValue::Number(42.0).to_display_string(), "42");
        assert_eq!(JsValue::Number(2.5).to_display_string(), "2.5");
        assert_eq!(JsValue::Number(f64::NAN).to_display_string(), "NaN");
        assert_eq!(JsValue::String("hola".into()).to_display_string(), "hola");
    }

    #[test]
    fn jsvalue_to_bool_truthy_falsy() {
        assert!(!JsValue::Undefined.to_bool());
        assert!(!JsValue::Null.to_bool());
        assert!(!JsValue::Bool(false).to_bool());
        assert!(!JsValue::Number(0.0).to_bool());
        assert!(!JsValue::Number(f64::NAN).to_bool());
        assert!(!JsValue::String("".into()).to_bool());
        assert!(JsValue::Bool(true).to_bool());
        assert!(JsValue::Number(-1.0).to_bool());
        assert!(JsValue::String("x".into()).to_bool());
    }

    #[test]
    fn objeto_coerce_a_string() {
        // Por ahora objetos vienen como su .toString() — `[object Object]`.
        let v = eval("({foo: 1})");
        match v {
            JsValue::String(s) => assert!(s.contains("object")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn console_log_captura_a_stdout() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.log('hola mundo')").expect("eval");
        assert_eq!(rt.stdout(), "hola mundo\n");
        assert!(rt.stderr().is_empty());
    }

    #[test]
    fn console_log_multiples_args() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.log('x', 42, true, null)").expect("eval");
        assert_eq!(rt.stdout(), "x 42 true null\n");
    }

    #[test]
    fn console_error_captura_a_stderr() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.error('boom')").expect("eval");
        assert!(rt.stdout().is_empty());
        assert_eq!(rt.stderr(), "boom\n");
    }

    #[test]
    fn console_log_acumula_entre_evals() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.log('a')").expect("e1");
        rt.eval("console.log('b')").expect("e2");
        rt.eval("console.log('c')").expect("e3");
        assert_eq!(rt.stdout(), "a\nb\nc\n");
    }

    #[test]
    fn clear_io_vacia_buffers() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.log('descartar')").expect("eval");
        assert!(!rt.stdout().is_empty());
        rt.clear_io();
        assert!(rt.stdout().is_empty());
        assert!(rt.stderr().is_empty());
    }

    #[test]
    fn console_log_objeto_es_json_stringify() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.log({a: 1, b: 'x'})").expect("eval");
        // JSON.stringify({a:1,b:'x'}) → {"a":1,"b":"x"}\n
        assert!(rt.stdout().contains("\"a\":1"));
        assert!(rt.stdout().contains("\"b\":\"x\""));
    }

    #[test]
    fn console_log_capturado_incluso_si_eval_falla_despues() {
        let mut rt = JsRuntime::new().expect("rt");
        let _ = rt.eval("console.log('antes del throw'); throw new Error('e')");
        // El throw NO debería quitar el log que ya se hizo.
        assert_eq!(rt.stdout(), "antes del throw\n");
    }

    #[test]
    fn set_document_define_title_y_url() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("Mi título", "https://example.com/x", "cuerpo")
            .expect("set_document");
        match rt.eval("document.title").expect("e") {
            JsValue::String(s) => assert_eq!(s, "Mi título"),
            other => panic!("{other:?}"),
        }
        match rt.eval("document.URL").expect("e") {
            JsValue::String(s) => assert_eq!(s, "https://example.com/x"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn set_document_define_body_textcontent() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "Hello world").expect("d");
        match rt.eval("document.body.textContent").expect("e") {
            JsValue::String(s) => assert_eq!(s, "Hello world"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn set_document_getElementById_devuelve_null() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        // Fase 7.2: stub que siempre devuelve null. El script puede
        // verificar la existencia de la función sin crashear.
        let v = rt.eval("document.getElementById('foo')").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn set_document_window_es_globalthis() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        let v = rt.eval("window === globalThis").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn set_document_escapa_strings_seguro() {
        // Strings con comillas, backslashes y newlines no deben romper
        // el script generado.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document(
            "Título \"con\" comillas",
            "https://x/y",
            "línea1\nlínea2\\foo",
        )
        .expect("d");
        let title = rt.eval("document.title").expect("e");
        match title {
            JsValue::String(s) => assert_eq!(s, "Título \"con\" comillas"),
            other => panic!("{other:?}"),
        }
        let body = rt.eval("document.body.textContent").expect("e");
        match body {
            JsValue::String(s) => assert_eq!(s, "línea1\nlínea2\\foo"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn js_string_literal_escapa_chars_basicos() {
        assert_eq!(js_string_literal("hola"), "\"hola\"");
        assert_eq!(js_string_literal("a\"b"), "\"a\\\"b\"");
        assert_eq!(js_string_literal("c\\d"), "\"c\\\\d\"");
        assert_eq!(js_string_literal("e\nf"), "\"e\\nf\"");
        assert_eq!(js_string_literal("g\tg"), "\"g\\tg\"");
    }

    #[test]
    fn js_string_literal_escapa_unicode_separators() {
        // U+2028 LINE SEPARATOR y U+2029 PARAGRAPH SEPARATOR son
        // legales en JSON pero rompen los parsers JS antiguos.
        let s = format!("a\u{2028}b\u{2029}c");
        let lit = js_string_literal(&s);
        assert!(lit.contains("\\u2028"));
        assert!(lit.contains("\\u2029"));
    }

    #[test]
    fn set_timeout_dispara_al_tick_correcto() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval("setTimeout(function(){ console.log('boom') }, 100)")
            .expect("registrar timeout");
        // Tick a t=50ms: aún no debe dispararse.
        let r = rt.tick(50).expect("tick 50");
        assert_eq!(r.fired, 0);
        assert_eq!(r.remaining, 1);
        assert!(rt.stdout().is_empty());
        // Tick a t=100ms: corresponde el fire_at exacto.
        let r = rt.tick(100).expect("tick 100");
        assert_eq!(r.fired, 1);
        assert_eq!(r.remaining, 0);
        assert_eq!(rt.stdout(), "boom\n");
    }

    #[test]
    fn set_interval_se_reprograma_y_dispara_repetido() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval("setInterval(function(){ console.log('t') }, 50)")
            .expect("registrar interval");
        let r1 = rt.tick(50).expect("tick 50");
        assert_eq!(r1.fired, 1);
        assert_eq!(r1.remaining, 1, "interval sigue vivo");
        let r2 = rt.tick(100).expect("tick 100");
        assert_eq!(r2.fired, 1);
        assert_eq!(r2.remaining, 1);
        let r3 = rt.tick(120).expect("tick 120");
        // 120 < 150, no debería dispararse aún.
        assert_eq!(r3.fired, 0);
        assert_eq!(r3.remaining, 1);
        assert_eq!(rt.stdout(), "t\nt\n");
    }

    #[test]
    fn clear_timeout_cancela_antes_de_fire() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval("var id = setTimeout(function(){ console.log('no') }, 100); clearTimeout(id);")
            .expect("registrar+cancelar");
        let r = rt.tick(200).expect("tick");
        assert_eq!(r.fired, 0);
        assert_eq!(r.remaining, 0);
        assert!(rt.stdout().is_empty());
    }

    #[test]
    fn clear_interval_para_el_loop() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval(
            "var id = setInterval(function(){ console.log('x') }, 10); \
             setTimeout(function(){ clearInterval(id) }, 25);",
        )
        .expect("registrar interval+timeout");
        // `__puriy_tick` dispara cada timer A LO SUMO una vez por tick
        // (no "catch-up" — matchea el comportamiento de browsers reales
        // cuando hay backlog). Así que en tick(30):
        //   - interval id=1 (fire_at=10) dispara una vez, reprograma a 40
        //   - timeout id=2 (fire_at=25) dispara y borra id=1
        let r = rt.tick(30).expect("tick 30");
        assert_eq!(r.fired, 2, "1 interval + 1 timeout cancelador");
        assert_eq!(r.remaining, 0, "clearInterval lo borró");
        assert_eq!(rt.stdout(), "x\n");
        // Tick siguiente: no debe disparar nada porque clearInterval
        // sacó el interval del queue.
        let r2 = rt.tick(100).expect("tick 100");
        assert_eq!(r2.fired, 0);
        assert_eq!(rt.stdout(), "x\n");
    }

    #[test]
    fn interval_no_hace_catch_up_por_tick() {
        // Si el host atrasa el poll (ej. 200ms con interval de 10ms), el
        // tick NO dispara 20 veces — sólo una vez, y reprograma al
        // siguiente. Esto matchea browsers reales (no spam de ticks
        // perdidos) y previene loops infinitos en setInterval(_, 0).
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval("setInterval(function(){ console.log('p') }, 10)")
            .expect("e");
        let r = rt.tick(200).expect("tick 200");
        assert_eq!(r.fired, 1);
        assert_eq!(r.remaining, 1);
        assert_eq!(rt.stdout(), "p\n");
    }

    #[test]
    fn callback_string_se_evalua_en_scope_global() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval("setTimeout('console.log(\"via string\")', 10)")
            .expect("registrar timeout con string");
        let r = rt.tick(10).expect("tick");
        assert_eq!(r.fired, 1);
        assert_eq!(rt.stdout(), "via string\n");
    }

    #[test]
    fn error_en_callback_no_crashea_el_tick() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval(
            "setTimeout(function(){ throw new Error('boom') }, 10); \
             setTimeout(function(){ console.log('sigo vivo') }, 20);",
        )
        .expect("registrar dos timers");
        let r = rt.tick(20).expect("tick");
        assert_eq!(r.fired, 2);
        assert_eq!(rt.stdout(), "sigo vivo\n");
        // El error fue capturado por el try/catch del __puriy_tick y
        // appendeado a __puriy_stderr.
        assert!(rt.stderr().contains("boom"), "stderr: {:?}", rt.stderr());
    }

    #[test]
    fn pending_timers_reporta_count_correcto() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        assert_eq!(rt.pending_timers(), 0);
        rt.eval("setTimeout(function(){}, 100); setTimeout(function(){}, 200);")
            .expect("e");
        assert_eq!(rt.pending_timers(), 2);
        rt.tick(100).expect("tick 100");
        assert_eq!(rt.pending_timers(), 1, "uno disparado, uno queda");
        rt.tick(200).expect("tick 200");
        assert_eq!(rt.pending_timers(), 0);
    }

    #[test]
    fn set_timeout_zero_dispara_al_proximo_tick() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("set_now");
        rt.eval("setTimeout(function(){ console.log('now') }, 0)")
            .expect("e");
        let r = rt.tick(0).expect("tick mismo instante");
        assert_eq!(r.fired, 1);
        assert_eq!(rt.stdout(), "now\n");
    }

    pub(super) fn snap(id: &str, tag: &str, text: &str) -> ElementSnapshot {
        ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: text.into(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }
    }

    pub(super) fn snap_with_class(id: &str, tag: &str, text: &str, class: &str) -> ElementSnapshot {
        ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: text.into(),
            class_list: vec![class.into()],
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }
    }

    pub(super) fn snap_with_value(id: &str, tag: &str, value: &str) -> ElementSnapshot {
        ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: Some(value.into()),
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }
    }

    pub(super) fn snap_with_parent(id: &str, tag: &str, parent_id: &str) -> ElementSnapshot {
        ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: Some(parent_id.into()),
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }
    }

    pub(super) fn snap_with_dataset(id: &str, tag: &str, dataset: &[(&str, &str)]) -> ElementSnapshot {
        // Reflejamos los data-* también en attributes — así un test que
        // construya un snapshot con `data-foo` puede leerlo tanto desde
        // `el.dataset.foo` como desde `el.getAttribute('data-foo')`.
        let attributes = dataset
            .iter()
            .map(|(k, v)| (format!("data-{}", k), v.to_string()))
            .collect();
        ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: dataset.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            attributes,
            dfs_index: 0,
        }
    }

    pub(super) fn snap_with_attrs(id: &str, tag: &str, attrs: &[(&str, &str)]) -> ElementSnapshot {
        ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            dfs_index: 0,
        }
    }

    // Helper de tests DOM-element: registra un elemento real en
    // __puriy_elements sin pasar por set_document (que reemplazaría el
    // `document` aumentado por los bootstraps fullscreen/pointerlock).
    pub(super) const MAKE_EL: &str =
        "var el = __puriy_make_element('el1', 'div', '', [], null, null, [], []); \
         __puriy_elements['el1'] = el;";

    // Helper: registra un service worker y deja la registration en `reg`.
    pub(super) const PUSH_REG: &str = "var reg = null; \
         navigator.serviceWorker.register('/sw.js').then(function(r){ reg = r; }); \
         __puriy_serviceworker_resolve(__puriy_sw_next_id - 1, '/');";

