//! Tests de AbortSignal, ReadableStream, FormData, Response/Request constructors, URL, Headers, File, crypto, structuredClone, URLSearchParams, Blob.bytes, sendBeacon, FileReader, queueMicrotask, AbortSignal.abort, DOMException, performance, crypto.subtle.
    use super::*;

    // ============= Fase 7.36 — AbortSignal.timeout / .any =============

    #[test]
    fn abort_signal_timeout_aborta_tras_ms() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("var s = AbortSignal.timeout(50)").expect("e");
        let v = rt.eval("s.aborted").expect("e");
        assert_eq!(v, JsValue::Bool(false));
        // Avanzamos el reloj 50ms — el setTimeout dispara y aborta.
        rt.tick(50).expect("tick");
        let v = rt.eval("s.aborted").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn abort_signal_timeout_rechaza_fetch() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var s = AbortSignal.timeout(10); var err = null; \
             fetch('/x', {signal: s}).catch(function(e) { err = e.message; });",
        )
        .expect("e");
        rt.tick(10).expect("tick");
        let v = rt.eval("err").expect("e");
        if let JsValue::String(s) = v {
            assert!(s.contains("AbortError"), "msg: {s}");
        } else {
            panic!("expected string, got {v:?}");
        }
    }

    #[test]
    fn abort_signal_any_aborta_cuando_cualquiera_aborta() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var c1 = new AbortController(); var c2 = new AbortController(); \
             var s = AbortSignal.any([c1.signal, c2.signal]); \
             c2.abort();",
        )
        .expect("e");
        let v = rt.eval("s.aborted").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn abort_signal_any_input_ya_aborted_nace_aborted() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var c = new AbortController(); c.abort(); \
             var s = AbortSignal.any([c.signal]);",
        )
        .expect("e");
        let v = rt.eval("s.aborted").expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn body_used_json_rechaza_text_posterior() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var err = null; \
             fetch('/x').then(function(r) { return r.json().then(function() { return r.text(); }); }) \
                        .catch(function(e) { err = e.message; });",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "{\"x\":1}", &[]).expect("r");
        let v = rt.eval("err").expect("e");
        if let JsValue::String(s) = v {
            assert!(s.contains("already read"), "msg: {s}");
        } else {
            panic!("expected string");
        }
    }

    // === Fase 7.45 — ReadableStream ===

    #[test]
    fn readable_stream_existe_y_es_constructor() {
        let mut rt = JsRuntime::new().expect("rt");
        let v = rt.eval("typeof ReadableStream").expect("e");
        assert_eq!(v, JsValue::String("function".into()));
        let v = rt
            .eval("new ReadableStream({}) instanceof ReadableStream")
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn readable_stream_enqueue_y_read_devuelve_chunk_luego_done() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var chunk = null; var done2 = null; \
             var s = new ReadableStream({ start: function(c) { c.enqueue('hola'); c.close(); } }); \
             var rd = s.getReader(); \
             rd.read().then(function(r) { chunk = r.value; \
                rd.read().then(function(r2) { done2 = r2.done; }); });",
        )
        .expect("e");
        // drain_pending_jobs ya corrió dentro de eval — leer los globals.
        assert_eq!(rt.eval("chunk").expect("e"), JsValue::String("hola".into()));
        assert_eq!(rt.eval("done2").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn readable_stream_getreader_dos_veces_tira_locked() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; var s = new ReadableStream({}); s.getReader(); \
             try { s.getReader(); } catch (e) { err = e.message; }",
        )
        .expect("e");
        let v = rt.eval("err").expect("e");
        if let JsValue::String(s) = v {
            assert!(s.contains("locked"), "msg: {s}");
        } else {
            panic!("expected string, got {v:?}");
        }
        assert_eq!(rt.eval("s.locked").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn readable_stream_pull_se_llama_lazy_al_leer() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; var vals = []; \
             var s = new ReadableStream({ pull: function(c) { \
                 n++; if (n <= 2) c.enqueue(n); else c.close(); } }); \
             var rd = s.getReader(); \
             rd.read().then(function(a) { vals.push(a.value); \
                rd.read().then(function(b) { vals.push(b.value); \
                   rd.read().then(function(d) { vals.push(d.done ? 'fin' : '?'); }); }); });",
        )
        .expect("e");
        assert_eq!(rt.eval("vals[0]").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("vals[1]").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("vals[2]").expect("e"), JsValue::String("fin".into()));
    }

    #[test]
    fn readable_stream_cancel_resuelve_y_llama_underlying_cancel() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var canceledWith = null; var resolved = false; \
             var s = new ReadableStream({ cancel: function(reason) { canceledWith = reason; } }); \
             s.cancel('porque si').then(function() { resolved = true; });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("canceledWith").expect("e"),
            JsValue::String("porque si".into())
        );
        assert_eq!(rt.eval("resolved").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn readable_stream_tee_alimenta_dos_branches() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var a = null; var b = null; \
             var s = new ReadableStream({ start: function(c) { c.enqueue('X'); c.close(); } }); \
             var pair = s.tee(); \
             pair[0].getReader().read().then(function(r) { a = r.value; }); \
             pair[1].getReader().read().then(function(r) { b = r.value; });",
        )
        .expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::String("X".into()));
        assert_eq!(rt.eval("b").expect("e"), JsValue::String("X".into()));
    }

    #[test]
    fn response_body_es_readable_stream() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var isStream = null; var same = null; \
             fetch('/x').then(function(r) { isStream = r.body instanceof ReadableStream; \
                same = (r.body === r.body); });",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "payload", &[]).expect("r");
        assert_eq!(rt.eval("isStream").expect("e"), JsValue::Bool(true));
        // El spec exige identidad: r.body === r.body (getter cacheado).
        assert_eq!(rt.eval("same").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn response_body_read_devuelve_bytes_del_body() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var bytes = null; var done2 = null; \
             fetch('/x').then(function(r) { var rd = r.body.getReader(); \
                rd.read().then(function(a) { bytes = Array.from(a.value); \
                   rd.read().then(function(c) { done2 = c.done; }); }); });",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "AB", &[]).expect("r");
        // 'A' = 65, 'B' = 66.
        assert_eq!(rt.eval("bytes[0]").expect("e"), JsValue::Number(65.0));
        assert_eq!(rt.eval("bytes[1]").expect("e"), JsValue::Number(66.0));
        assert_eq!(rt.eval("done2").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn response_body_leido_marca_body_used_y_text_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var err = null; \
             fetch('/x').then(function(r) { var rd = r.body.getReader(); \
                rd.read().then(function() { \
                   r.text().catch(function(e) { err = e.message; }); }); });",
        )
        .expect("e");
        rt.resolve_fetch(1, 200, "OK", "datos", &[]).expect("r");
        let v = rt.eval("err").expect("e");
        if let JsValue::String(s) = v {
            assert!(s.contains("already read"), "msg: {s}");
        } else {
            panic!("expected string, got {v:?}");
        }
    }

    #[test]
    fn readable_stream_async_iterator_recorre_chunks() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var collected = []; \
             var s = new ReadableStream({ start: function(c) { \
                 c.enqueue('a'); c.enqueue('b'); c.enqueue('c'); c.close(); } }); \
             (async function() { for await (const ch of s) { collected.push(ch); } })();",
        )
        .expect("e");
        assert_eq!(rt.eval("collected.length").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("collected[0]").expect("e"), JsValue::String("a".into()));
        assert_eq!(rt.eval("collected[2]").expect("e"), JsValue::String("c".into()));
    }

    // ============= Fase 7.54 — FormData =============

    #[test]
    fn formdata_append_get_getall() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var f = new FormData(); f.append('a', '1'); f.append('a', '2'); f.append('b', 'x');",
        )
        .expect("e");
        assert_eq!(rt.eval("f.get('a')").expect("e"), JsValue::String("1".into()));
        assert_eq!(rt.eval("f.getAll('a').join(',')").expect("e"), JsValue::String("1,2".into()));
        assert_eq!(rt.eval("f.has('b')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("f.get('z')").expect("e"), JsValue::Null);
    }

    #[test]
    fn formdata_set_reemplaza_y_delete() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var f = new FormData(); f.append('a', '1'); f.append('a', '2'); \
             f.set('a', '9'); f.append('b', 'y'); f.delete('b');",
        )
        .expect("e");
        assert_eq!(rt.eval("f.getAll('a').join(',')").expect("e"), JsValue::String("9".into()));
        assert_eq!(rt.eval("f.has('b')").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn formdata_itera_y_acepta_blob() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var f = new FormData(); f.append('k', 'v'); \
             f.append('file', new Blob(['hola']), 'a.txt'); \
             var seq = []; for (var p of f) { seq.push(p[0]); } \
             var blobOk = f.get('file') instanceof Blob;",
        )
        .expect("e");
        assert_eq!(rt.eval("seq.join(',')").expect("e"), JsValue::String("k,file".into()));
        assert_eq!(rt.eval("blobOk").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.55 — Response constructor =============

    #[test]
    fn response_constructor_status_y_text() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = new Response('cuerpo', { status: 201, statusText: 'Created' }); \
             var st = r.status; var ok = r.ok; var stt = r.statusText; var txt = null; \
             r.text().then(function(t) { txt = t; });",
        )
        .expect("e");
        assert_eq!(rt.eval("st").expect("e"), JsValue::Number(201.0));
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("stt").expect("e"), JsValue::String("Created".into()));
        assert_eq!(rt.eval("txt").expect("e"), JsValue::String("cuerpo".into()));
    }

    #[test]
    fn response_json_static_y_blob() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = Response.json({ n: 7 }); var ct = r.headers.get('content-type'); \
             var parsed = null; r.text().then(function(t) { parsed = JSON.parse(t).n; }); \
             var r2 = new Response('xy', { headers: { 'content-type': 'text/plain' } }); \
             var bt = null, bsz = null; \
             r2.blob().then(function(b) { bt = b.type; bsz = b.size; });",
        )
        .expect("e");
        assert_eq!(rt.eval("ct").expect("e"), JsValue::String("application/json".into()));
        assert_eq!(rt.eval("parsed").expect("e"), JsValue::Number(7.0));
        assert_eq!(rt.eval("bt").expect("e"), JsValue::String("text/plain".into()));
        assert_eq!(rt.eval("bsz").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn response_clone_preserva_body_y_bloquea_si_usado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = new Response('dato'); var c = r.clone(); \
             var a = null, b = null; \
             r.text().then(function(t) { a = t; }); \
             c.text().then(function(t) { b = t; }); \
             var threw = false; try { r.clone(); } catch (e) { threw = true; }",
        )
        .expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::String("dato".into()));
        assert_eq!(rt.eval("b").expect("e"), JsValue::String("dato".into()));
        // r ya fue consumido por .text() → clone() debe tirar.
        assert_eq!(rt.eval("threw").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.56 — Request constructor + fetch(Request) =============

    #[test]
    fn request_constructor_campos_y_clone() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var req = new Request('https://api.test/x', { method: 'post', \
                headers: { 'X-A': '1' }, body: 'cuerpo' }); \
             var m = req.method; var u = req.url; var h = req.headers.get('x-a'); \
             var bodyTxt = null; req.text().then(function(t) { bodyTxt = t; });",
        )
        .expect("e");
        assert_eq!(rt.eval("m").expect("e"), JsValue::String("POST".into()));
        assert_eq!(rt.eval("u").expect("e"), JsValue::String("https://api.test/x".into()));
        assert_eq!(rt.eval("h").expect("e"), JsValue::String("1".into()));
        assert_eq!(rt.eval("bodyTxt").expect("e"), JsValue::String("cuerpo".into()));
    }

    #[test]
    fn fetch_acepta_request_object() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var req = new Request('/api/y', { method: 'PUT', body: 'payload' }); \
             fetch(req);",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "PUT");
        assert_eq!(parts[2], "https://example.com/api/y");
        assert_eq!(parts[4], "payload");
    }

    #[test]
    fn fetch_request_init_pisa_al_request() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var req = new Request('/z', { method: 'GET' }); \
             fetch(req, { method: 'DELETE' });",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "DELETE");
    }

    // ============= Fase 7.57 — serialización de body (multipart + auto CT) =============

    #[test]
    fn fetch_formdata_se_serializa_a_multipart() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var f = new FormData(); f.append('campo', 'valor'); \
             f.append('archivo', new Blob(['hola']), 'a.txt'); \
             fetch('/up', { method: 'POST', body: f });",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        // [4] body multipart; los pares de header van aplanados desde [5].
        assert!(parts[4].contains("Content-Disposition: form-data; name=\"campo\""));
        assert!(parts[4].contains("filename=\"a.txt\""));
        assert!(parts[4].contains("valor"));
        // El Content-Type con boundary sólo aparece en la región de headers.
        let headers = parts[5..].join("\u{001D}");
        assert!(headers.contains("multipart/form-data; boundary=----puriyFormBoundary"));
    }

    #[test]
    fn fetch_urlsearchparams_auto_content_type() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("fetch('/api', { method: 'POST', body: new URLSearchParams({ k: 'v' }) });")
            .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[4], "k=v");
        let headers = parts[5..].join("\u{001D}");
        assert!(headers.contains("application/x-www-form-urlencoded"));
    }

    #[test]
    fn fetch_content_type_explicito_no_se_pisa() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "fetch('/api', { method: 'POST', \
                headers: { 'Content-Type': 'application/json' }, \
                body: new URLSearchParams({ k: 'v' }) });",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        let headers = parts[5..].join("\u{001D}");
        assert!(headers.contains("application/json"));
        assert!(!headers.contains("x-www-form-urlencoded"));
    }

    #[test]
    fn xhr_formdata_se_serializa_a_multipart() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var f = new FormData(); f.append('a', '1'); \
             var x = new XMLHttpRequest(); x.open('POST', '/up'); x.send(f);",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert!(parts[4].contains("Content-Disposition: form-data; name=\"a\""));
        let headers = parts[5..].join("\u{001D}");
        assert!(headers.contains("multipart/form-data; boundary="));
    }

    // ============= Fase 7.58 — new URL(url, base) =============

    #[test]
    fn url_constructor_parsea_componentes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var u = new URL('https://user:pass@host.com:8080/a/b?x=1&y=2#frag');",
        )
        .expect("e");
        assert_eq!(rt.eval("u.protocol").expect("e"), JsValue::String("https:".into()));
        assert_eq!(rt.eval("u.hostname").expect("e"), JsValue::String("host.com".into()));
        assert_eq!(rt.eval("u.port").expect("e"), JsValue::String("8080".into()));
        assert_eq!(rt.eval("u.host").expect("e"), JsValue::String("host.com:8080".into()));
        assert_eq!(rt.eval("u.username").expect("e"), JsValue::String("user".into()));
        assert_eq!(rt.eval("u.password").expect("e"), JsValue::String("pass".into()));
        assert_eq!(rt.eval("u.pathname").expect("e"), JsValue::String("/a/b".into()));
        assert_eq!(rt.eval("u.search").expect("e"), JsValue::String("?x=1&y=2".into()));
        assert_eq!(rt.eval("u.hash").expect("e"), JsValue::String("#frag".into()));
        assert_eq!(rt.eval("u.origin").expect("e"), JsValue::String("https://host.com:8080".into()));
        assert_eq!(rt.eval("u.searchParams.get('y')").expect("e"), JsValue::String("2".into()));
    }

    #[test]
    fn url_constructor_resuelve_relativa_con_base() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var u = new URL('/api?q=1', 'https://example.com/dir/page.html');")
            .expect("e");
        assert_eq!(
            rt.eval("u.href").expect("e"),
            JsValue::String("https://example.com/api?q=1".into())
        );
        // Relativa de path con colapso de `..` (reusa __puriy_normalize_path).
        rt.eval("var u2 = new URL('../x.json', 'https://example.com/a/b/page.html');")
            .expect("e");
        assert_eq!(
            rt.eval("u2.href").expect("e"),
            JsValue::String("https://example.com/a/x.json".into())
        );
    }

    #[test]
    fn url_searchparams_modifica_href() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var u = new URL('https://h.com/p?a=1'); u.searchParams.set('a', '9'); \
             u.searchParams.append('b', '2');",
        )
        .expect("e");
        assert_eq!(rt.eval("u.search").expect("e"), JsValue::String("?a=9&b=2".into()));
        assert_eq!(
            rt.eval("u.href").expect("e"),
            JsValue::String("https://h.com/p?a=9&b=2".into())
        );
    }

    #[test]
    fn url_constructor_sin_scheme_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var threw = false; try { new URL('/no-base'); } catch (e) { threw = true; }")
            .expect("e");
        assert_eq!(rt.eval("threw").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn url_estaticos_object_url_se_preservan() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var b = new Blob(['z']); var ou = URL.createObjectURL(b); \
             var resuelto = globalThis.__puriy_resolve_blob_url(ou) === b; \
             URL.revokeObjectURL(ou); \
             var tras = globalThis.__puriy_resolve_blob_url(ou) === null; \
             var esConstructor = typeof URL === 'function';",
        )
        .expect("e");
        assert_eq!(rt.eval("resuelto").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("tras").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("esConstructor").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.59 — Headers iterable completo =============

    #[test]
    fn headers_entries_y_symbol_iterator() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var h = new Headers({ 'Content-Type': 'text/html', 'X-Foo': 'bar' }); \
             var seq = []; for (var pair of h) { seq.push(pair[0] + '=' + pair[1]); }",
        )
        .expect("e");
        // Iteración ordenada por nombre (lowercased): content-type < x-foo.
        assert_eq!(
            rt.eval("seq.join('&')").expect("e"),
            JsValue::String("content-type=text/html&x-foo=bar".into())
        );
    }

    #[test]
    fn headers_values_y_spread() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var h = new Headers(); h.set('a', '1'); h.set('b', '2'); \
             var vals = []; var it = h.values(); var n = it.next(); \
             while (!n.done) { vals.push(n.value); n = it.next(); } \
             var pares = [...h].length;",
        )
        .expect("e");
        assert_eq!(rt.eval("vals.join(',')").expect("e"), JsValue::String("1,2".into()));
        assert_eq!(rt.eval("pares").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn headers_alimenta_urlsearchparams() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var h = new Headers({ 'x': '1', 'y': '2' }); \
             var p = new URLSearchParams(h); var s = p.toString();",
        )
        .expect("e");
        // URLSearchParams consume el iterable de pares de Headers.
        assert_eq!(rt.eval("s").expect("e"), JsValue::String("x=1&y=2".into()));
    }

    // ============= Fase 7.60 — File (subclase de Blob) =============

    #[test]
    fn file_constructor_es_blob_y_tiene_name() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var f = new File(['hola'], 'a.txt', { type: 'text/plain', lastModified: 5 }); \
             var esBlob = f instanceof Blob; var esFile = f instanceof File; \
             var nm = f.name; var tp = f.type; var sz = f.size; var lm = f.lastModified; \
             var txt = null; f.text().then(function(t) { txt = t; });",
        )
        .expect("e");
        assert_eq!(rt.eval("esBlob").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("esFile").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("nm").expect("e"), JsValue::String("a.txt".into()));
        assert_eq!(rt.eval("tp").expect("e"), JsValue::String("text/plain".into()));
        assert_eq!(rt.eval("sz").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("lm").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("txt").expect("e"), JsValue::String("hola".into()));
    }

    #[test]
    fn file_hereda_metodos_de_blob() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var f = new File(['abcdef'], 'b.bin'); \
             var sl = f.slice(1, 3); var slEsBlob = sl instanceof Blob; \
             var sub = null; sl.text().then(function(t) { sub = t; });",
        )
        .expect("e");
        assert_eq!(rt.eval("slEsBlob").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("sub").expect("e"), JsValue::String("bc".into()));
    }

    #[test]
    fn formdata_blob_se_envuelve_en_file() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var fd = new FormData(); \
             fd.append('doc', new Blob(['x'], { type: 'text/plain' }), 'd.txt'); \
             fd.append('texto', 'plano'); \
             var v = fd.get('doc'); var esFile = v instanceof File; var nm = v.name; \
             var planoEsString = typeof fd.get('texto') === 'string';",
        )
        .expect("e");
        assert_eq!(rt.eval("esFile").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("nm").expect("e"), JsValue::String("d.txt".into()));
        assert_eq!(rt.eval("planoEsString").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.61 — URL.parse / URL.canParse =============

    #[test]
    fn url_parse_devuelve_url_o_null() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ok = URL.parse('https://example.com/a?x=1'); \
             var host = ok ? ok.hostname : null; var path = ok ? ok.pathname : null; \
             var malo = URL.parse('/sin-base'); var esNull = malo === null;",
        )
        .expect("e");
        assert_eq!(rt.eval("host").expect("e"), JsValue::String("example.com".into()));
        assert_eq!(rt.eval("path").expect("e"), JsValue::String("/a".into()));
        assert_eq!(rt.eval("esNull").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn url_parse_resuelve_relativa_con_base() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var u = URL.parse('../x.json', 'https://example.com/a/b/page.html'); \
             var href = u ? u.href : null;",
        )
        .expect("e");
        assert_eq!(
            rt.eval("href").expect("e"),
            JsValue::String("https://example.com/a/x.json".into())
        );
    }

    #[test]
    fn url_can_parse_da_booleano() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var bueno = URL.canParse('https://example.com'); \
             var conBase = URL.canParse('/p', 'https://example.com'); \
             var malo = URL.canParse('/sin-base');",
        )
        .expect("e");
        assert_eq!(rt.eval("bueno").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("conBase").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("malo").expect("e"), JsValue::Bool(false));
    }

    // ===== Fase 7.62 — Response.redirect + Headers.getSetCookie =====

    #[test]
    fn response_redirect_setea_location_y_status() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = Response.redirect('https://example.com/x', 301); \
             var st = r.status; var loc = r.headers.get('location'); \
             var def = Response.redirect('/y'); var defSt = def.status;",
        )
        .expect("e");
        assert_eq!(rt.eval("st").expect("e"), JsValue::Number(301.0));
        assert_eq!(rt.eval("loc").expect("e"), JsValue::String("https://example.com/x".into()));
        assert_eq!(rt.eval("defSt").expect("e"), JsValue::Number(302.0));
    }

    #[test]
    fn response_redirect_status_invalido_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var threw = false; \
             try { Response.redirect('https://e.com', 200); } catch (e) { threw = true; }",
        )
        .expect("e");
        assert_eq!(rt.eval("threw").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn headers_get_set_cookie_lista_separada() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var h = new Headers(); \
             h.append('Set-Cookie', 'a=1; Path=/'); \
             h.append('Set-Cookie', 'b=2; HttpOnly'); \
             h.append('X-Other', 'z'); \
             var cookies = h.getSetCookie(); var n = cookies.length; \
             var joined = h.get('set-cookie');",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("cookies[0]").expect("e"), JsValue::String("a=1; Path=/".into()));
        assert_eq!(rt.eval("cookies[1]").expect("e"), JsValue::String("b=2; HttpOnly".into()));
        // get() sí los comma-joina (comportamiento legacy preservado).
        assert_eq!(
            rt.eval("joined").expect("e"),
            JsValue::String("a=1; Path=/, b=2; HttpOnly".into())
        );
    }

    // ===== Fase 7.63 — Response.formData / Request.formData =====

    #[test]
    fn response_formdata_urlencoded() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = new Response('a=1&a=2&b=x', { \
                 headers: { 'content-type': 'application/x-www-form-urlencoded' } }); \
             var na = null, nb = null, all = null; \
             r.formData().then(function(fd) { \
                 na = fd.get('a'); all = fd.getAll('a').join(','); nb = fd.get('b'); });",
        )
        .expect("e");
        assert_eq!(rt.eval("na").expect("e"), JsValue::String("1".into()));
        assert_eq!(rt.eval("all").expect("e"), JsValue::String("1,2".into()));
        assert_eq!(rt.eval("nb").expect("e"), JsValue::String("x".into()));
    }

    #[test]
    fn response_formdata_multipart_round_trip() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var f = new FormData(); f.append('campo', 'valor'); \
             f.append('archivo', new Blob(['hola'], { type: 'text/plain' }), 'a.txt'); \
             var ser = globalThis.__puriy_serialize_body(f); \
             var r = new Response(ser.text, { headers: { 'content-type': ser.contentType } }); \
             var campo = null, esFile = null, nm = null, contenido = null; \
             r.formData().then(function(fd) { \
                 campo = fd.get('campo'); \
                 var a = fd.get('archivo'); esFile = a instanceof File; nm = a.name; \
                 a.text().then(function(t) { contenido = t; }); });",
        )
        .expect("e");
        assert_eq!(rt.eval("campo").expect("e"), JsValue::String("valor".into()));
        assert_eq!(rt.eval("esFile").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("nm").expect("e"), JsValue::String("a.txt".into()));
        assert_eq!(rt.eval("contenido").expect("e"), JsValue::String("hola".into()));
    }

    #[test]
    fn request_formdata_parsea_urlencoded() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var req = new Request('https://e.com', { method: 'POST', \
                 headers: { 'content-type': 'application/x-www-form-urlencoded' }, \
                 body: 'k=v&n=7' }); \
             var k = null, n = null; \
             req.formData().then(function(fd) { k = fd.get('k'); n = fd.get('n'); });",
        )
        .expect("e");
        assert_eq!(rt.eval("k").expect("e"), JsValue::String("v".into()));
        assert_eq!(rt.eval("n").expect("e"), JsValue::String("7".into()));
    }

    // ===== Fase 7.64 — crypto.getRandomValues / crypto.randomUUID =====

    #[test]
    fn crypto_random_uuid_formato_y_unicidad() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var u = crypto.randomUUID(); \
             var re = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/; \
             var ok = re.test(u); \
             var distintos = crypto.randomUUID() !== crypto.randomUUID();",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("distintos").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn crypto_get_random_values_llena_y_devuelve_la_misma() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var a = new Uint8Array(16); var ret = crypto.getRandomValues(a); \
             var mismaRef = ret === a; var len = a.length; \
             var enRango = true; var algunoNoCero = false; \
             for (var i = 0; i < a.length; i++) { \
                 if (a[i] < 0 || a[i] > 255 || (a[i] | 0) !== a[i]) enRango = false; \
                 if (a[i] !== 0) algunoNoCero = true; \
             }",
        )
        .expect("e");
        assert_eq!(rt.eval("mismaRef").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("len").expect("e"), JsValue::Number(16.0));
        assert_eq!(rt.eval("enRango").expect("e"), JsValue::Bool(true));
        // Prob. de los 16 bytes en cero es ~256^-16; en la práctica nunca.
        assert_eq!(rt.eval("algunoNoCero").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn crypto_get_random_values_excede_cuota_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        // El chequeo de cuota (65536 bytes) ocurre ANTES del loop de llenado,
        // así que evaluar 65537 elementos tira sin gastar fuel en el fill.
        rt.eval(
            "var threw = false; \
             try { crypto.getRandomValues(new Uint8Array(65537)); } catch (e) { threw = true; }",
        )
        .expect("e");
        assert_eq!(rt.eval("threw").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.65 — structuredClone =============

    #[test]
    fn structured_clone_copia_profunda_independiente() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var orig = { a: 1, b: { c: [1, 2, 3] } }; \
             var copia = structuredClone(orig); \
             orig.b.c[0] = 99; orig.a = 7; \
             var sigueUno = copia.a === 1; \
             var arrIntacto = copia.b.c.join(',') === '1,2,3'; \
             var distintoRef = copia.b !== orig.b;",
        )
        .expect("e");
        assert_eq!(rt.eval("sigueUno").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("arrIntacto").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("distintoRef").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn structured_clone_preserva_refs_compartidas_y_ciclos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var hijo = { v: 1 }; var orig = { x: hijo, y: hijo }; \
             orig.self = orig; \
             var c = structuredClone(orig); \
             var refCompartida = c.x === c.y; \
             var cicloOk = c.self === c; \
             var noAliasOriginal = c.x !== hijo;",
        )
        .expect("e");
        assert_eq!(rt.eval("refCompartida").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("cicloOk").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("noAliasOriginal").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn structured_clone_tipos_especiales_y_funcion_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var orig = { \
                 d: new Date(1000), \
                 m: new Map([['k', 1]]), \
                 s: new Set([4, 5]), \
                 ta: new Uint8Array([7, 8, 9]) }; \
             var c = structuredClone(orig); \
             var dateOk = (c.d instanceof Date) && c.d.getTime() === 1000 && c.d !== orig.d; \
             var mapOk = (c.m instanceof Map) && c.m.get('k') === 1 && c.m !== orig.m; \
             var setOk = (c.s instanceof Set) && c.s.has(5) && c.s !== orig.s; \
             var taOk = (c.ta instanceof Uint8Array) && c.ta[1] === 8 && c.ta !== orig.ta; \
             var fnTira = false; \
             try { structuredClone(function() {}); } catch (e) { fnTira = true; }",
        )
        .expect("e");
        assert_eq!(rt.eval("dateOk").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("mapOk").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("setOk").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("taOk").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("fnTira").expect("e"), JsValue::Bool(true));
    }

    // ===== Fase 7.66 — URLSearchParams.size + has/delete dos args =====

    #[test]
    fn usp_size_cuenta_pares() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var p = new URLSearchParams('a=1&a=2&b=3'); var n0 = p.size; \
             p.append('c', '4'); var n1 = p.size; \
             p.delete('a'); var n2 = p.size;",
        )
        .expect("e");
        assert_eq!(rt.eval("n0").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("n1").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("n2").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn usp_has_y_delete_de_dos_args() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var p = new URLSearchParams('a=1&a=2&b=3'); \
             var hasA2 = p.has('a', '2'); var hasA9 = p.has('a', '9'); var hasA = p.has('a'); \
             p.delete('a', '1'); \
             var queda = p.getAll('a').join(','); var size = p.size;",
        )
        .expect("e");
        assert_eq!(rt.eval("hasA2").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("hasA9").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("hasA").expect("e"), JsValue::Bool(true));
        // delete('a','1') sólo borra el par a=1; queda a=2 y b=3.
        assert_eq!(rt.eval("queda").expect("e"), JsValue::String("2".into()));
        assert_eq!(rt.eval("size").expect("e"), JsValue::Number(2.0));
    }

    // ===== Fase 7.67 — .bytes() en Blob / Response / Request =====

    #[test]
    fn bytes_en_blob_response_request() {
        let mut rt = JsRuntime::new().expect("rt");
        // Los `.then` corren como microtasks que drenan al CERRAR el eval, así
        // que las aserciones van en evals separados (el patrón del resto).
        rt.eval(
            "var bb = null, rb = null, qb = null, rusada = false; \
             new Blob(['AB']).bytes().then(function(u) { bb = u; }); \
             var r = new Response('CD'); r.bytes().then(function(u) { rb = u; }); \
             r.text().then(function() {}, function() { rusada = true; }); \
             var q = new Request('https://e.com', { method: 'POST', body: 'EF' }); \
             q.bytes().then(function(u) { qb = u; });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("(bb instanceof Uint8Array) && bb[0] === 65 && bb[1] === 66").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(
            rt.eval("(rb instanceof Uint8Array) && rb[0] === 67 && rb[1] === 68").expect("e"),
            JsValue::Bool(true)
        );
        // bytes() consumió el body → text() posterior rechaza (bodyUsed).
        assert_eq!(rt.eval("rusada").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("(qb instanceof Uint8Array) && qb[0] === 69 && qb[1] === 70").expect("e"),
            JsValue::Bool(true)
        );
    }

    // ===== Fase 7.68 — navigator.sendBeacon =====

    #[test]
    fn navigator_send_beacon_encola_post() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("var ret = navigator.sendBeacon('/log', 'evento=click');")
            .expect("e");
        assert_eq!(rt.eval("ret").expect("e"), JsValue::Bool(true));
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        let parts: Vec<&str> = muts[0].value.split('\u{001D}').collect();
        assert_eq!(parts[1], "POST");
        assert_eq!(parts[2], "https://example.com/log");
        assert_eq!(parts[3], "1");
        assert_eq!(parts[4], "evento=click");
    }

    #[test]
    fn navigator_user_agent_y_online() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ua = typeof navigator.userAgent === 'string'; var on = navigator.onLine;")
            .expect("e");
        assert_eq!(rt.eval("ua").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("on").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.69 — FileReader =============

    #[test]
    fn filereader_read_as_text_y_data_url() {
        let mut rt = JsRuntime::new().expect("rt");
        // Los eventos disparan en un microtask (drena al cerrar el eval): se
        // agenda en el primer eval y se asierta en evals separados.
        rt.eval(
            "var txt = null, durl = null, estado = null; \
             var b = new Blob(['Hi'], { type: 'text/plain' }); \
             var fr = new FileReader(); fr.onload = function() { txt = fr.result; estado = fr.readyState; }; \
             fr.readAsText(b); \
             var fr2 = new FileReader(); fr2.addEventListener('load', function() { durl = fr2.result; }); \
             fr2.readAsDataURL(b);",
        )
        .expect("e");
        assert_eq!(rt.eval("txt").expect("e"), JsValue::String("Hi".into()));
        assert_eq!(rt.eval("estado").expect("e"), JsValue::Number(2.0));
        assert_eq!(
            rt.eval("durl").expect("e"),
            JsValue::String("data:text/plain;base64,SGk=".into())
        );
    }

    #[test]
    fn filereader_read_as_array_buffer_y_binary_string() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var abLen = null, abByte0 = null, bin = null; \
             var b = new Blob([new Uint8Array([65, 0, 66])]); \
             var fr = new FileReader(); \
             fr.onload = function() { abLen = fr.result.byteLength; \
                 abByte0 = new Uint8Array(fr.result)[0]; }; \
             fr.readAsArrayBuffer(b); \
             var fr2 = new FileReader(); fr2.onload = function() { bin = fr2.result; }; \
             fr2.readAsBinaryString(new Blob(['AB']));",
        )
        .expect("e");
        assert_eq!(rt.eval("abLen").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("abByte0").expect("e"), JsValue::Number(65.0));
        assert_eq!(rt.eval("bin").expect("e"), JsValue::String("AB".into()));
    }

    #[test]
    fn filereader_loadstart_load_loadend_en_orden_y_no_blob_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var orden = []; var b = new Blob(['x']); \
             var fr = new FileReader(); \
             fr.addEventListener('loadstart', function() { orden.push('start'); }); \
             fr.addEventListener('load', function() { orden.push('load'); }); \
             fr.addEventListener('loadend', function() { orden.push('end'); }); \
             fr.readAsText(b); \
             var tira = false; try { new FileReader().readAsText('no soy blob'); } catch (e) { tira = true; }",
        )
        .expect("e");
        assert_eq!(rt.eval("orden.join(',')").expect("e"), JsValue::String("start,load,end".into()));
        assert_eq!(rt.eval("tira").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.70 — queueMicrotask =============

    #[test]
    fn queue_microtask_corre_callback() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var corrio = false; queueMicrotask(function() { corrio = true; });")
            .expect("e");
        assert_eq!(rt.eval("corrio").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn queue_microtask_preserva_fifo_con_promesas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var orden = []; \
             queueMicrotask(function() { orden.push('a'); }); \
             Promise.resolve().then(function() { orden.push('b'); }); \
             queueMicrotask(function() { orden.push('c'); });",
        )
        .expect("e");
        // Las tres microtasks corren en orden de encolado (FIFO).
        assert_eq!(rt.eval("orden.join(',')").expect("e"), JsValue::String("a,b,c".into()));
    }

    #[test]
    fn queue_microtask_no_funcion_tira() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var tira = false; try { queueMicrotask(123); } catch (e) { tira = true; }")
            .expect("e");
        assert_eq!(rt.eval("tira").expect("e"), JsValue::Bool(true));
    }

    // ============= Fase 7.71 — AbortSignal.abort() static =============

    #[test]
    fn abort_signal_abort_static_nace_aborted() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var s = AbortSignal.abort(); var ab = s.aborted; \
             var tira = false; try { s.throwIfAborted(); } catch (e) { tira = true; }",
        )
        .expect("e");
        assert_eq!(rt.eval("ab").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("tira").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn abort_signal_abort_con_reason_propaga() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var s = AbortSignal.abort('boom'); var r = s.reason;")
            .expect("e");
        assert_eq!(rt.eval("r").expect("e"), JsValue::String("boom".into()));
    }

    #[test]
    fn abort_signal_abort_static_rechaza_fetch_inmediato() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval(
            "var rechazo = null; \
             fetch('/x', { signal: AbortSignal.abort() }) \
                 .then(function() {}, function(e) { rechazo = String(e); });",
        )
        .expect("e");
        // El signal ya-abortado hace que fetch rechace sin tocar la red.
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 0);
        match rt.eval("rechazo").expect("e") {
            JsValue::String(s) => assert!(s.contains("AbortError"), "esperaba AbortError, fue {s}"),
            other => panic!("esperaba string de rechazo, fue {other:?}"),
        }
    }

    // ============= Fase 7.72 — DOMException =============

    #[test]
    fn dom_exception_construct_name_message_code() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var e = new DOMException('algo falló', 'AbortError'); \
             var nm = e.name; var msg = e.message; var code = e.code; \
             var esError = e instanceof Error; var esDom = e instanceof DOMException;",
        )
        .expect("e");
        assert_eq!(rt.eval("nm").expect("e"), JsValue::String("AbortError".into()));
        assert_eq!(rt.eval("msg").expect("e"), JsValue::String("algo falló".into()));
        assert_eq!(rt.eval("code").expect("e"), JsValue::Number(20.0));
        assert_eq!(rt.eval("esError").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("esDom").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn dom_exception_default_name_y_constantes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var e = new DOMException('m'); var nm = e.name; var code = e.code; \
             var ab = DOMException.ABORT_ERR; var dc = DOMException.DATA_CLONE_ERR;",
        )
        .expect("e");
        // Nombre no-legacy → code 0; default name 'Error'.
        assert_eq!(rt.eval("nm").expect("e"), JsValue::String("Error".into()));
        assert_eq!(rt.eval("code").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("ab").expect("e"), JsValue::Number(20.0));
        assert_eq!(rt.eval("dc").expect("e"), JsValue::Number(25.0));
    }

    #[test]
    fn dom_exception_tostring_y_throw() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var s = String(new DOMException('boom', 'DataCloneError')); \
             var caught = null; \
             try { throw new DOMException('no está', 'NotFoundError'); } \
             catch (e) { caught = e.name + '/' + e.code; }",
        )
        .expect("e");
        assert_eq!(rt.eval("s").expect("e"), JsValue::String("DataCloneError: boom".into()));
        assert_eq!(rt.eval("caught").expect("e"), JsValue::String("NotFoundError/8".into()));
    }

    // ===== Fase 7.73 — Request init fields + Response.redirected =====

    #[test]
    fn request_init_fields_defaults() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = new Request('https://e.com'); \
             var c = r.cache, rd = r.redirect, ref = r.referrer, \
                 rp = r.referrerPolicy, integ = r.integrity, ka = r.keepalive;",
        )
        .expect("e");
        assert_eq!(rt.eval("c").expect("e"), JsValue::String("default".into()));
        assert_eq!(rt.eval("rd").expect("e"), JsValue::String("follow".into()));
        assert_eq!(rt.eval("ref").expect("e"), JsValue::String("about:client".into()));
        assert_eq!(rt.eval("rp").expect("e"), JsValue::String("".into()));
        assert_eq!(rt.eval("integ").expect("e"), JsValue::String("".into()));
        assert_eq!(rt.eval("ka").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn request_init_fields_explicitos_y_clone_pisa() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = new Request('https://e.com', { cache: 'no-store', redirect: 'manual', \
                 integrity: 'sha256-x', keepalive: true }); \
             var c = r.cache, rd = r.redirect, integ = r.integrity, ka = r.keepalive; \
             var r2 = new Request(r, { cache: 'reload' }); \
             var c2 = r2.cache, rd2 = r2.redirect;",
        )
        .expect("e");
        assert_eq!(rt.eval("c").expect("e"), JsValue::String("no-store".into()));
        assert_eq!(rt.eval("rd").expect("e"), JsValue::String("manual".into()));
        assert_eq!(rt.eval("integ").expect("e"), JsValue::String("sha256-x".into()));
        assert_eq!(rt.eval("ka").expect("e"), JsValue::Bool(true));
        // El init del segundo Request pisa cache pero hereda redirect del input.
        assert_eq!(rt.eval("c2").expect("e"), JsValue::String("reload".into()));
        assert_eq!(rt.eval("rd2").expect("e"), JsValue::String("manual".into()));
    }

    #[test]
    fn response_redirected_default_false_y_clone() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = new Response('x'); var d0 = r.redirected; \
             r.redirected = true; var c = r.clone(); var dc = c.redirected;",
        )
        .expect("e");
        assert_eq!(rt.eval("d0").expect("e"), JsValue::Bool(false));
        // clone() preserva el flag.
        assert_eq!(rt.eval("dc").expect("e"), JsValue::Bool(true));
    }

    // ===== Fase 7.74 — performance.now() / timeOrigin =====

    #[test]
    fn performance_now_y_time_origin() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(123).expect("now");
        rt.eval("var t = performance.now(); var o = performance.timeOrigin; var esNum = typeof t === 'number';")
            .expect("e");
        assert_eq!(rt.eval("t").expect("e"), JsValue::Number(123.0));
        assert_eq!(rt.eval("o").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("esNum").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn performance_now_avanza_con_el_reloj() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(0).expect("now0");
        assert_eq!(rt.eval("performance.now()").expect("e"), JsValue::Number(0.0));
        rt.set_now_ms(500).expect("now500");
        assert_eq!(rt.eval("performance.now()").expect("e"), JsValue::Number(500.0));
    }

    // ===== Fase 7.75 — crypto.subtle.digest (SHA-256 / SHA-1) =====

    // Helper JS para hexear un ArrayBuffer en una global `hex`.
    const HEX_HELPER: &str = "function __hex(buf){var v=new Uint8Array(buf),s='';\
        for(var i=0;i<v.length;i++){var h=v[i].toString(16);if(h.length<2)h='0'+h;s+=h;}return s;}";

    #[test]
    fn subtle_digest_sha256_vectores() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(HEX_HELPER).expect("helper");
        // Un SHA-256 cuesta ~100M de fuel (interpretado sobre wasmi), así que
        // cada digest va en su propio eval con el fuel recargado antes.
        rt.eval(
            "var hAbc = null; \
             crypto.subtle.digest('SHA-256', new TextEncoder().encode('abc')) \
                 .then(function(b) { hAbc = __hex(b); });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("hAbc").expect("e"),
            JsValue::String("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into())
        );
        rt.set_fuel(DEFAULT_FUEL);
        rt.eval(
            "var hEmpty = null; \
             crypto.subtle.digest('SHA-256', new Uint8Array([])) \
                 .then(function(b) { hEmpty = __hex(b); });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("hEmpty").expect("e"),
            JsValue::String("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into())
        );
    }

    #[test]
    fn subtle_digest_sha1_vector() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(HEX_HELPER).expect("helper");
        rt.eval(
            "var h = null; \
             crypto.subtle.digest('SHA-1', new TextEncoder().encode('abc')) \
                 .then(function(b) { h = __hex(b); });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("h").expect("e"),
            JsValue::String("a9993e364706816aba3e25717850c26c9cd0d89d".into())
        );
    }

    #[test]
    fn subtle_digest_acepta_objeto_algoritmo_y_rechaza_no_soportado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(HEX_HELPER).expect("helper");
        rt.eval(
            "var hObj = null, rechazo = null, rechazoData = null; \
             crypto.subtle.digest({ name: 'SHA-256' }, new TextEncoder().encode('abc')) \
                 .then(function(b) { hObj = __hex(b); }); \
             crypto.subtle.digest('SHA-512', new Uint8Array([1])) \
                 .then(function() {}, function(e) { rechazo = String(e); }); \
             crypto.subtle.digest('SHA-256', 'soy un string') \
                 .then(function() {}, function(e) { rechazoData = String(e); });",
        )
        .expect("e");
        // El algoritmo como objeto {name} también funciona.
        assert_eq!(
            rt.eval("hObj").expect("e"),
            JsValue::String("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into())
        );
        match rt.eval("rechazo").expect("e") {
            JsValue::String(s) => assert!(s.contains("NotSupportedError"), "fue {s}"),
            other => panic!("esperaba rechazo, fue {other:?}"),
        }
        match rt.eval("rechazoData").expect("e") {
            JsValue::String(s) => assert!(s.contains("BufferSource"), "fue {s}"),
            other => panic!("esperaba rechazo de data, fue {other:?}"),
        }
    }

