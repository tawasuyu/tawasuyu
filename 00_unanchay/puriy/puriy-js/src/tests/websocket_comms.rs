//! Tests de EventTarget, eventos tipados, WebSocket, EventSource, BroadcastChannel, MessageChannel, ErrorEvent, PromiseRejectionEvent, window/self, navigator básico, online/offline, Location, History.
    use super::*;

    // ============= Fase 7.76 — EventTarget genérico =============

    #[test]
    fn event_target_add_dispatch_remove() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var et = new EventTarget(); var hits = 0, tgtOk = false; \
             var fn = function(e) { hits++; tgtOk = (e.target === et); }; \
             et.addEventListener('ping', fn); \
             et.dispatchEvent(new Event('ping')); \
             et.removeEventListener('ping', fn); \
             et.dispatchEvent(new Event('ping'));",
        )
        .expect("e");
        // Disparó una vez (la segunda ya sin listener).
        assert_eq!(rt.eval("hits").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("tgtOk").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn event_target_once_y_dedup() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var et = new EventTarget(); var a = 0, b = 0; \
             var fa = function() { a++; }; \
             et.addEventListener('x', fa); et.addEventListener('x', fa); /* dedup */ \
             et.addEventListener('x', function() { b++; }, { once: true }); \
             et.dispatchEvent(new Event('x')); \
             et.dispatchEvent(new Event('x'));",
        )
        .expect("e");
        // fa registrado una sola vez (dedup) → 2 dispatches = 2 hits.
        assert_eq!(rt.eval("a").expect("e"), JsValue::Number(2.0));
        // el listener once corre sólo en el primer dispatch.
        assert_eq!(rt.eval("b").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn event_target_handle_event_stop_immediate_y_default_prevented() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var et = new EventTarget(); var seq = []; \
             et.addEventListener('y', { handleEvent: function(e) { seq.push('obj'); e.stopImmediatePropagation(); } }); \
             et.addEventListener('y', function() { seq.push('no-deberia'); }); \
             var ev = new Event('y', { cancelable: true }); \
             var et2 = new EventTarget(); \
             et2.addEventListener('z', function(e) { e.preventDefault(); }); \
             var ret = et2.dispatchEvent(new Event('z', { cancelable: true })); \
             et.dispatchEvent(ev);",
        )
        .expect("e");
        // handleEvent corrió y stopImmediatePropagation cortó al segundo listener.
        assert_eq!(rt.eval("seq.join(',')").expect("e"), JsValue::String("obj".into()));
        // dispatchEvent devuelve false cuando un listener llamó preventDefault.
        assert_eq!(rt.eval("ret").expect("e"), JsValue::Bool(false));
    }

    // ============= Fase 7.77 — eventos tipados (Message/Close/Progress) =============

    #[test]
    fn message_event_campos_y_es_event() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ev = new MessageEvent('message', { data: { x: 7 }, origin: 'http://a', lastEventId: '5' }); \
             var dx = ev.data.x; var org = ev.origin; var lid = ev.lastEventId; \
             var esEvent = (ev instanceof Event); var esMsg = (ev instanceof MessageEvent); \
             var tipo = ev.type;",
        )
        .expect("e");
        assert_eq!(rt.eval("dx").expect("e"), JsValue::Number(7.0));
        assert_eq!(rt.eval("org").expect("e"), JsValue::String("http://a".into()));
        assert_eq!(rt.eval("lid").expect("e"), JsValue::String("5".into()));
        assert_eq!(rt.eval("esEvent").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("esMsg").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("tipo").expect("e"), JsValue::String("message".into()));
    }

    #[test]
    fn close_event_campos_y_defaults() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var a = new CloseEvent('close', { code: 1000, reason: 'bye', wasClean: true }); \
             var ac = a.code, ar = a.reason, aw = a.wasClean, aEv = (a instanceof Event); \
             var b = new CloseEvent('close'); \
             var bc = b.code, br = b.reason, bw = b.wasClean;",
        )
        .expect("e");
        assert_eq!(rt.eval("ac").expect("e"), JsValue::Number(1000.0));
        assert_eq!(rt.eval("ar").expect("e"), JsValue::String("bye".into()));
        assert_eq!(rt.eval("aw").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("aEv").expect("e"), JsValue::Bool(true));
        // Defaults sin init.
        assert_eq!(rt.eval("bc").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("br").expect("e"), JsValue::String("".into()));
        assert_eq!(rt.eval("bw").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn progress_event_campos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ev = new ProgressEvent('progress', { lengthComputable: true, loaded: 5, total: 10 }); \
             var lc = ev.lengthComputable, ld = ev.loaded, tt = ev.total, esEv = (ev instanceof Event);",
        )
        .expect("e");
        assert_eq!(rt.eval("lc").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ld").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("tt").expect("e"), JsValue::Number(10.0));
        assert_eq!(rt.eval("esEv").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.78 — WebSocket =============

    #[test]
    fn websocket_construye_y_encola_open() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var ws = new WebSocket('wss://echo.example/sock', ['p1', 'p2']); \
             var rs = ws.readyState; var u = ws.url; \
             var cc = WebSocket.CONNECTING; var oo = WebSocket.OPEN; \
             var esET = (ws instanceof EventTarget);",
        )
        .expect("e");
        assert_eq!(rt.eval("rs").expect("e"), JsValue::Number(0.0)); // CONNECTING
        assert_eq!(rt.eval("cc").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("oo").expect("e"), JsValue::Number(1.0));
        assert_eq!(
            rt.eval("u").expect("e"),
            JsValue::String("wss://echo.example/sock".into())
        );
        assert_eq!(rt.eval("esET").expect("e"), JsValue::Bool(true));
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "websocket");
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "open");
        assert_eq!(parts[2], "wss://echo.example/sock");
        assert_eq!(parts[3], "p1,p2");
    }

    #[test]
    fn websocket_send_antes_de_open_tira_y_dispatch_open_message() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var abrio = false, recibido = null; \
             var ws = new WebSocket('ws://x/'); \
             ws.onopen = function() { abrio = true; }; \
             ws.onmessage = function(e) { recibido = e.data; }; \
             var tiroAlEnviarTemprano = false; \
             try { ws.send('temprano'); } catch (e) { tiroAlEnviarTemprano = true; } \
             __puriy_ws_dispatch(ws._id, 'open', 'p1', ''); \
             var rsTrasOpen = ws.readyState; \
             ws.send('hola'); \
             __puriy_ws_dispatch(ws._id, 'message', 'mundo');",
        )
        .expect("e");
        assert_eq!(
            rt.eval("tiroAlEnviarTemprano").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(rt.eval("abrio").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("rsTrasOpen").expect("e"), JsValue::Number(1.0)); // OPEN
        assert_eq!(
            rt.eval("recibido").expect("e"),
            JsValue::String("mundo".into())
        );
    }

    #[test]
    fn websocket_close_transiciona_y_close_event() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var cerro = null; \
             var ws = new WebSocket('ws://x/'); \
             ws.addEventListener('close', function(e) { cerro = { c: e.code, r: e.reason, w: e.wasClean }; }); \
             __puriy_ws_dispatch(ws._id, 'open'); \
             ws.close(1000, 'bye'); \
             var rsCerrando = ws.readyState; \
             __puriy_ws_dispatch(ws._id, 'close', 1000, 'bye', '1'); \
             var rsFinal = ws.readyState;",
        )
        .expect("e");
        assert_eq!(rt.eval("rsCerrando").expect("e"), JsValue::Number(2.0)); // CLOSING
        assert_eq!(rt.eval("rsFinal").expect("e"), JsValue::Number(3.0)); // CLOSED
        assert_eq!(rt.eval("cerro.c").expect("e"), JsValue::Number(1000.0));
        assert_eq!(rt.eval("cerro.r").expect("e"), JsValue::String("bye".into()));
        assert_eq!(rt.eval("cerro.w").expect("e"), JsValue::Bool(true));
        // El close() del cliente encoló una mutación 'close'.
        let muts = rt.drain_dom_mutations();
        let cierre = muts.iter().find(|m| {
            m.kind == "websocket" && m.value.split('\u{001D}').nth(1) == Some("close")
        });
        assert!(cierre.is_some());
    }

    // ============= Fase 7.79 — EventSource (SSE) =============

    #[test]
    fn eventsource_construye_y_encola_open() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var es = new EventSource('/stream'); \
             var rs = es.readyState; var u = es.url; \
             var cc = EventSource.CONNECTING, oo = EventSource.OPEN, xx = EventSource.CLOSED; \
             var esET = (es instanceof EventTarget);",
        )
        .expect("e");
        assert_eq!(rt.eval("rs").expect("e"), JsValue::Number(0.0)); // CONNECTING
        assert_eq!(rt.eval("cc").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("oo").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("xx").expect("e"), JsValue::Number(2.0));
        assert_eq!(
            rt.eval("u").expect("e"),
            JsValue::String("https://example.com/stream".into())
        );
        assert_eq!(rt.eval("esET").expect("e"), JsValue::Bool(true));
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "eventsource");
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "open");
        assert_eq!(parts[2], "https://example.com/stream");
        assert_eq!(parts[3], "0"); // withCredentials false
    }

    #[test]
    fn eventsource_dispatch_open_message_y_named() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var abrio = false, msg = null, lid = null, named = null, onmsgEnNamed = false; \
             var es = new EventSource('/s'); \
             es.onopen = function() { abrio = true; }; \
             es.onmessage = function(e) { msg = e.data; lid = e.lastEventId; }; \
             es.addEventListener('update', function(e) { named = e.data; }); \
             __puriy_es_dispatch(es._id, 'open'); \
             var rsTrasOpen = es.readyState; \
             __puriy_es_dispatch(es._id, 'message', '', 'hola', '7'); \
             __puriy_es_dispatch(es._id, 'message', 'update', 'parche'); \
             if (msg === 'parche') onmsgEnNamed = true;",
        )
        .expect("e");
        assert_eq!(rt.eval("abrio").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("rsTrasOpen").expect("e"), JsValue::Number(1.0)); // OPEN
        assert_eq!(rt.eval("msg").expect("e"), JsValue::String("hola".into()));
        assert_eq!(rt.eval("lid").expect("e"), JsValue::String("7".into()));
        // El evento nombrado 'update' va sólo a su listener.
        assert_eq!(rt.eval("named").expect("e"), JsValue::String("parche".into()));
        // ...y NO disparó onmessage (que sigue en 'hola').
        assert_eq!(rt.eval("onmsgEnNamed").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn es_dispatch_metodo_host_entrega_open_y_message() {
        // El wrapper Rust `JsRuntime::es_dispatch` (el que llama el worker del
        // chrome) llega al listener igual que el hook crudo.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var log = []; var es = new EventSource('/s'); \
             es.onopen = function() { log.push('open:' + es.readyState); }; \
             es.onmessage = function(e) { log.push('msg:' + e.data + ':' + e.lastEventId); };",
        )
        .expect("e");
        let id = match rt.eval("es._id").expect("e") {
            JsValue::Number(n) => n as u32,
            other => panic!("es._id no es número: {other:?}"),
        };
        rt.es_dispatch(id, "open", "", "", "").expect("open");
        rt.es_dispatch(id, "message", "message", "hola", "7").expect("msg");
        assert_eq!(
            rt.eval("log.join('|')").expect("e"),
            JsValue::String("open:1|msg:hola:7".into())
        );
    }

    #[test]
    fn eventsource_close_detiene_dispatch() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var msgs = 0; \
             var es = new EventSource('/s'); \
             es.onmessage = function() { msgs++; }; \
             __puriy_es_dispatch(es._id, 'open'); \
             __puriy_es_dispatch(es._id, 'message', '', 'uno'); \
             es.close(); \
             var rs = es.readyState; \
             __puriy_es_dispatch(es._id, 'message', '', 'dos');",
        )
        .expect("e");
        assert_eq!(rt.eval("rs").expect("e"), JsValue::Number(2.0)); // CLOSED
        // Tras close() el registry se vació: el segundo dispatch es no-op.
        assert_eq!(rt.eval("msgs").expect("e"), JsValue::Number(1.0));
        let muts = rt.drain_dom_mutations();
        let cierre = muts.iter().find(|m| {
            m.kind == "eventsource" && m.value.split('\u{001D}').nth(1) == Some("close")
        });
        assert!(cierre.is_some());
    }

    // ---- Fase 7.80 — BroadcastChannel ----

    #[test]
    fn broadcast_channel_entrega_a_otros_no_al_emisor() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var aGot = false, bData = null, cData = null; \
             var a = new BroadcastChannel('sala'); \
             var b = new BroadcastChannel('sala'); \
             var c = new BroadcastChannel('otra'); \
             a.onmessage = function() { aGot = true; }; \
             b.onmessage = function(e) { bData = e.data; }; \
             c.onmessage = function(e) { cData = e.data; }; \
             a.postMessage('hola');",
        )
        .expect("e");
        // El emisor NO recibe su propio mensaje.
        assert_eq!(rt.eval("aGot").expect("e"), JsValue::Bool(false));
        // Otro canal del mismo name sí.
        assert_eq!(rt.eval("bData").expect("e"), JsValue::String("hola".into()));
        // Un canal de otro name no recibe nada.
        assert_eq!(rt.eval("cData").expect("e"), JsValue::Null);
    }

    #[test]
    fn broadcast_channel_close_deja_de_recibir_y_tira_post_close() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var a = new BroadcastChannel('x'); \
             var b = new BroadcastChannel('x'); \
             b.onmessage = function() { n++; }; \
             a.postMessage('uno'); \
             b.close(); \
             a.postMessage('dos'); \
             var tiro = false; \
             try { b.postMessage('z'); } catch (e) { tiro = (e.name === 'InvalidStateError'); }",
        )
        .expect("e");
        // b recibió sólo el primero; tras close() no recibe más.
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
        // postMessage sobre un canal cerrado tira InvalidStateError.
        assert_eq!(rt.eval("tiro").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn broadcast_channel_addeventlistener_y_es_eventtarget() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var vistos = []; \
             var a = new BroadcastChannel('g'); \
             var b = new BroadcastChannel('g'); \
             var esET = (a instanceof EventTarget); \
             b.addEventListener('message', function(e) { vistos.push(e.data); }); \
             a.postMessage('m1'); \
             a.postMessage('m2');",
        )
        .expect("e");
        assert_eq!(rt.eval("esET").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("vistos.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("vistos[0]").expect("e"), JsValue::String("m1".into()));
        assert_eq!(rt.eval("vistos[1]").expect("e"), JsValue::String("m2".into()));
    }

    // ---- Fase 7.81 — MessageChannel + MessagePort ----

    #[test]
    fn message_channel_ida_y_vuelta_via_onmessage() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var got = null, back = null; \
             var mc = new MessageChannel(); \
             mc.port2.onmessage = function(e) { got = e.data; }; \
             mc.port1.onmessage = function(e) { back = e.data; }; \
             mc.port1.postMessage('ida'); \
             mc.port2.postMessage('vuelta');",
        )
        .expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("ida".into()));
        assert_eq!(rt.eval("back").expect("e"), JsValue::String("vuelta".into()));
    }

    #[test]
    fn message_channel_mensajes_pre_start_se_encolan() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var recibidos = []; \
             var mc = new MessageChannel(); \
             mc.port1.postMessage('a'); \
             mc.port1.postMessage('b'); \
             mc.port2.onmessage = function(e) { recibidos.push(e.data); };",
        )
        .expect("e");
        // Encolados antes de arrancar port2; al setear onmessage (start
        // implícito) se entregan en orden.
        assert_eq!(rt.eval("recibidos.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("recibidos[0]").expect("e"), JsValue::String("a".into()));
        assert_eq!(rt.eval("recibidos[1]").expect("e"), JsValue::String("b".into()));
    }

    #[test]
    fn message_channel_close_corta_entrega_y_es_eventtarget() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var mc = new MessageChannel(); \
             var esET = (mc.port1 instanceof EventTarget); \
             mc.port2.onmessage = function() { n++; }; \
             mc.port1.postMessage('uno'); \
             mc.port2.close(); \
             mc.port1.postMessage('dos');",
        )
        .expect("e");
        assert_eq!(rt.eval("esET").expect("e"), JsValue::Bool(true));
        // Tras close() en port2, port1.postMessage es no-op.
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.82 — ErrorEvent + reportError ----

    #[test]
    fn report_error_dispara_evento_error_y_es_event() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var msg = null, esEE = false, esE = false; \
             addEventListener('error', function(ev) { \
                 msg = ev.message; \
                 esEE = (ev instanceof ErrorEvent); \
                 esE = (ev instanceof Event); \
             }); \
             reportError(new TypeError('boom'));",
        )
        .expect("e");
        assert_eq!(rt.eval("msg").expect("e"), JsValue::String("boom".into()));
        assert_eq!(rt.eval("esEE").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("esE").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn report_error_invoca_onerror_con_firma_clasica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var capturado = null, nargs = 0; \
             globalThis.onerror = function(message, filename, lineno, colno, error) { \
                 capturado = message; nargs = arguments.length; return true; \
             }; \
             reportError('caída');",
        )
        .expect("e");
        // onerror recibe el message como primer arg (no el event) y los 5 args.
        assert_eq!(rt.eval("capturado").expect("e"), JsValue::String("caída".into()));
        assert_eq!(rt.eval("nargs").expect("e"), JsValue::Number(5.0));
    }

    #[test]
    fn error_event_campos_y_defaults() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var e = new ErrorEvent('error', { message: 'x', lineno: 7, colno: 3, error: 42 }); \
             var d = new ErrorEvent('error');",
        )
        .expect("e");
        assert_eq!(rt.eval("e.message").expect("e"), JsValue::String("x".into()));
        assert_eq!(rt.eval("e.lineno").expect("e"), JsValue::Number(7.0));
        assert_eq!(rt.eval("e.colno").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("e.error").expect("e"), JsValue::Number(42.0));
        // Defaults: message vacío, lineno/colno 0, error null.
        assert_eq!(rt.eval("d.message").expect("e"), JsValue::String("".into()));
        assert_eq!(rt.eval("d.lineno").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("d.error").expect("e"), JsValue::Null);
    }

    // ---- Fase 7.83 — PromiseRejectionEvent + unhandledrejection ----

    #[test]
    fn unhandled_rejection_dispara_evento_con_reason_y_promise() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var visto = null, esPRE = false, esE = false; \
             var p = Promise.reject('x').catch(function(){}); \
             addEventListener('unhandledrejection', function(ev) { \
                 visto = ev.reason; \
                 esPRE = (ev instanceof PromiseRejectionEvent); \
                 esE = (ev instanceof Event); \
                 ev.preventDefault(); \
             }); \
             __puriy_emit_unhandled_rejection('motivo', p);",
        )
        .expect("e");
        assert_eq!(rt.eval("visto").expect("e"), JsValue::String("motivo".into()));
        assert_eq!(rt.eval("esPRE").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("esE").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn unhandled_rejection_sin_handler_loguea_a_stderr() {
        let mut rt = JsRuntime::new().expect("rt");
        // Sin preventDefault → cae al log por defecto en stderr.
        rt.eval("__puriy_emit_unhandled_rejection(new Error('zap'));").expect("e");
        let err = rt.stderr();
        assert!(err.contains("Uncaught (in promise)"), "stderr: {err}");
        assert!(err.contains("zap"), "stderr: {err}");
    }

    #[test]
    fn unhandled_rejection_preventdefault_suprime_log() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "onunhandledrejection = function(ev) { ev.preventDefault(); }; \
             __puriy_emit_unhandled_rejection('silenciado');",
        )
        .expect("e");
        // preventDefault desde el handler `on…` ⇒ no se loguea nada.
        assert!(!rt.stderr().contains("Uncaught"), "stderr: {}", rt.stderr());
    }

    #[test]
    fn rejection_handled_despacha_a_su_listener() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             addEventListener('rejectionhandled', function(ev) { \
                 if (ev.type === 'rejectionhandled') n++; \
             }); \
             __puriy_emit_rejection_handled(null);",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.84 — window / self como alias del global ----

    #[test]
    fn window_y_self_son_el_global() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("window === globalThis").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("self === globalThis").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("window === self").expect("e"), JsValue::Bool(true));
        // Auto-referencias cerradas.
        assert_eq!(rt.eval("window.window === window").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("self.self === self").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("typeof window !== 'undefined'").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn window_ve_props_definidas_en_globalthis() {
        let mut rt = JsRuntime::new().expect("rt");
        // Una API que vive en globalThis (console) se ve por el alias window.
        assert_eq!(rt.eval("typeof window.console").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("window.setTimeout === setTimeout").expect("e"), JsValue::Bool(true));
        // Lo nuevo agregado por código de usuario en window aparece en globalThis.
        rt.eval("window.miFlag = 42;").expect("e");
        assert_eq!(rt.eval("globalThis.miFlag").expect("e"), JsValue::Number(42.0));
    }

    #[test]
    fn window_jerarquia_de_navegacion_colapsa_en_el_global() {
        let mut rt = JsRuntime::new().expect("rt");
        // Sin iframes: parent/top son el propio global y length = 0.
        assert_eq!(rt.eval("window.parent === window").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("window.top === window").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("window.frames === window").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("window.length").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.85 — navigator ampliado ----

    #[test]
    fn navigator_props_de_feature_detection() {
        let mut rt = JsRuntime::new().expect("rt");
        // Locale + capacidades.
        assert_eq!(rt.eval("navigator.language").expect("e"), JsValue::String("es-ES".into()));
        assert_eq!(rt.eval("Array.isArray(navigator.languages)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.languages[0]").expect("e"), JsValue::String("es-ES".into()));
        assert_eq!(rt.eval("navigator.hardwareConcurrency >= 1").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.cookieEnabled").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.maxTouchPoints").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn navigator_constantes_legacy() {
        let mut rt = JsRuntime::new().expect("rt");
        // Valores literales que el spec obliga a devolver en todo browser.
        assert_eq!(rt.eval("navigator.appCodeName").expect("e"), JsValue::String("Mozilla".into()));
        assert_eq!(rt.eval("navigator.appName").expect("e"), JsValue::String("Netscape".into()));
        assert_eq!(rt.eval("navigator.product").expect("e"), JsValue::String("Gecko".into()));
    }

    // ---- Fase 7.86 — eventos online/offline ----

    #[test]
    fn set_online_dispara_offline_y_online_y_actualiza_navigator() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var log = []; \
             addEventListener('offline', function() { log.push('off:' + navigator.onLine); }); \
             addEventListener('online', function() { log.push('on:' + navigator.onLine); }); \
             __puriy_set_online(false); \
             __puriy_set_online(true);",
        )
        .expect("e");
        // navigator.onLine refleja el último estado y el evento ve el valor ya actualizado.
        assert_eq!(rt.eval("navigator.onLine").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("log.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("log[0]").expect("e"), JsValue::String("off:false".into()));
        assert_eq!(rt.eval("log[1]").expect("e"), JsValue::String("on:true".into()));
    }

    #[test]
    fn set_online_sin_cambio_es_noop() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             addEventListener('online', function() { n++; }); \
             var r = __puriy_set_online(true);",
        )
        .expect("e");
        // Arranca online; setear online de nuevo no dispara nada.
        assert_eq!(rt.eval("r").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.87 — Location object ----

    #[test]
    fn location_componentes_se_parsean() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/a/b?q=1#frag", "b").expect("d");
        assert_eq!(rt.eval("location.protocol").expect("e"), JsValue::String("https:".into()));
        assert_eq!(rt.eval("location.host").expect("e"), JsValue::String("example.com".into()));
        assert_eq!(rt.eval("location.pathname").expect("e"), JsValue::String("/a/b".into()));
        assert_eq!(rt.eval("location.search").expect("e"), JsValue::String("?q=1".into()));
        assert_eq!(rt.eval("location.hash").expect("e"), JsValue::String("#frag".into()));
        assert_eq!(
            rt.eval("location.origin").expect("e"),
            JsValue::String("https://example.com".into())
        );
    }

    #[test]
    fn location_hash_setter_dispara_hashchange_sin_navegar() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/p", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var ev = null; \
             addEventListener('hashchange', function(e) { ev = e.newURL; }); \
             location.hash = 'seccion';",
        )
        .expect("e");
        // hash normaliza con '#', dispara hashchange, location.hash refleja.
        assert_eq!(rt.eval("location.hash").expect("e"), JsValue::String("#seccion".into()));
        assert_eq!(
            rt.eval("ev").expect("e"),
            JsValue::String("https://example.com/p#seccion".into())
        );
        // same-document: NO publica navegación al chrome.
        let muts = rt.drain_dom_mutations();
        assert!(muts.iter().all(|m| m.kind != "navigate"));
    }

    #[test]
    fn location_assign_publica_navegacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/p", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("location.assign('/otra')").expect("e");
        let muts = rt.drain_dom_mutations();
        let nav = muts.iter().find(|m| m.kind == "navigate").expect("entry navigate");
        let parts: Vec<&str> = nav.value.split('\u{001D}').collect();
        assert_eq!(parts[0], "push");
        assert_eq!(parts[1], "https://example.com/otra");
        // location.href se actualiza de inmediato (spec).
        assert_eq!(
            rt.eval("location.href").expect("e"),
            JsValue::String("https://example.com/otra".into())
        );
    }

    // ---- Fase 7.88 — History API ----

    #[test]
    fn history_pushstate_actualiza_length_state_y_location() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        let base = rt.eval("history.length").expect("e");
        let base = if let JsValue::Number(n) = base { n } else { 0.0 };
        rt.eval(
            "history.pushState({page:1}, '', '/uno'); \
             history.pushState({page:2}, '', '/dos');",
        )
        .expect("e");
        assert_eq!(rt.eval("history.length").expect("e"), JsValue::Number(base + 2.0));
        assert_eq!(rt.eval("history.state.page").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("location.pathname").expect("e"), JsValue::String("/dos".into()));
    }

    #[test]
    fn history_back_dispara_popstate_con_state_previo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var seq = []; \
             addEventListener('popstate', function(e) { seq.push(e.state ? e.state.page : -1); }); \
             history.pushState({page:1}, '', '/uno'); \
             history.pushState({page:2}, '', '/dos'); \
             history.back();",
        )
        .expect("e");
        assert_eq!(rt.eval("seq.length").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("seq[0]").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("location.pathname").expect("e"), JsValue::String("/uno".into()));
        assert_eq!(rt.eval("history.state.page").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn history_replacestate_no_crece_pila_ni_dispara_popstate() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        let base = rt.eval("history.length").expect("e");
        rt.eval(
            "var n = 0; addEventListener('popstate', function() { n++; }); \
             history.replaceState({r:1}, '', '/reemplazo');",
        )
        .expect("e");
        // replaceState pisa el entry actual: misma longitud, sin popstate.
        assert_eq!(rt.eval("history.length").expect("e"), base);
        assert_eq!(rt.eval("history.state.r").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("location.pathname").expect("e"), JsValue::String("/reemplazo".into()));
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

