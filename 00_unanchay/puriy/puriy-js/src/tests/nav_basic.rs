//! Tests de NetworkInformation, cookie, Cache API, StorageEvent, Permissions, Notification, Geolocation, Clipboard, WebShare, matchMedia, screen, ServiceWorker, MediaDevices, BatteryManager, WakeLock, StorageManager.
    use super::*;

    // ---- Fase 7.89 — navigator.connection (NetworkInformation) ----

    #[test]
    fn connection_props_y_es_eventtarget() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("navigator.connection.effectiveType").expect("e"),
            JsValue::String("4g".into())
        );
        assert_eq!(
            rt.eval("typeof navigator.connection.rtt").expect("e"),
            JsValue::String("number".into())
        );
        assert_eq!(rt.eval("navigator.connection.saveData").expect("e"), JsValue::Bool(false));
        assert_eq!(
            rt.eval("navigator.connection instanceof EventTarget").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(
            rt.eval("navigator.mozConnection === navigator.connection").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn set_connection_actualiza_y_dispara_change() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var seq = []; \
             navigator.connection.onchange = function() { seq.push('on:' + navigator.connection.effectiveType); }; \
             navigator.connection.addEventListener('change', function() { seq.push('al:' + navigator.connection.saveData); }); \
             var r = __puriy_set_connection({ effectiveType: '2g', saveData: true, rtt: 300 });",
        )
        .expect("e");
        assert_eq!(rt.eval("r").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("navigator.connection.effectiveType").expect("e"),
            JsValue::String("2g".into())
        );
        assert_eq!(rt.eval("navigator.connection.rtt").expect("e"), JsValue::Number(300.0));
        // onchange (handler) + addEventListener('change') corren ambos, en orden.
        assert_eq!(rt.eval("seq.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("seq[0]").expect("e"), JsValue::String("on:2g".into()));
        assert_eq!(rt.eval("seq[1]").expect("e"), JsValue::String("al:true".into()));
    }

    // ---- Fase 7.90 — document.cookie ----

    #[test]
    fn cookie_set_y_get_round_trip() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval("document.cookie = 'a=1'; document.cookie = 'b=2';").expect("e");
        assert_eq!(rt.eval("document.cookie").expect("e"), JsValue::String("a=1; b=2".into()));
        // re-set del mismo nombre actualiza el valor, no duplica.
        rt.eval("document.cookie = 'a=9';").expect("e");
        assert_eq!(rt.eval("document.cookie").expect("e"), JsValue::String("a=9; b=2".into()));
    }

    #[test]
    fn cookie_max_age_cero_borra() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval("document.cookie = 'tmp=x'; document.cookie = 'keep=y';").expect("e");
        rt.eval("document.cookie = 'tmp=; Max-Age=0';").expect("e");
        assert_eq!(rt.eval("document.cookie").expect("e"), JsValue::String("keep=y".into()));
    }

    #[test]
    fn cookie_httponly_de_red_no_es_visible_a_js() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        // Cookie HttpOnly inyectada por la red: el jar la guarda pero JS no la ve.
        rt.eval(
            "__puriy_set_cookie_from_network('sid=secreto; HttpOnly; Path=/'); \
             document.cookie = 'vis=1';",
        )
        .expect("e");
        assert_eq!(rt.eval("document.cookie").expect("e"), JsValue::String("vis=1".into()));
        // sanity: el jar guardó ambas (sid no es visible, pero existe).
        assert_eq!(
            rt.eval("Object.keys(__puriy_cookie_jar).sort().join(',')").expect("e"),
            JsValue::String("sid,vis".into())
        );
    }

    // ---- Fase 7.91 — Cache API (caches / CacheStorage) ----

    #[test]
    fn caches_open_put_match_round_trip() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var got = null, hadMiss = null; \
             caches.open('v1').then(function(c) { \
                 return c.put('/data', new Response('hola', { status: 200 })).then(function() { \
                     return c.match('/data'); \
                 }); \
             }).then(function(resp) { return resp.text(); }).then(function(t) { got = t; }) \
              .then(function() { return caches.open('v1'); }) \
              .then(function(c) { return c.match('/ausente'); }) \
              .then(function(r) { hadMiss = (r === undefined); });",
        )
        .expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("hola".into()));
        assert_eq!(rt.eval("hadMiss").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn caches_keys_has_y_delete() {
        let mut rt = JsRuntime::new().expect("rt");
        // Cadena única para que el orden de microtasks sea determinista.
        rt.eval(
            "var r = {}; \
             caches.open('v1').then(function() { return caches.open('v2'); }) \
              .then(function() { return caches.keys(); }).then(function(k) { r.names = k.join(','); }) \
              .then(function() { return caches.has('v1'); }).then(function(h) { r.hasV1 = h; }) \
              .then(function() { return caches.delete('v1'); }).then(function(d) { r.delOk = d; }) \
              .then(function() { return caches.has('v1'); }).then(function(h) { r.hasAfter = h; });",
        )
        .expect("e");
        assert_eq!(rt.eval("r.names").expect("e"), JsValue::String("v1,v2".into()));
        assert_eq!(rt.eval("r.hasV1").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("r.delOk").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("r.hasAfter").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn cache_matchall_y_cachestorage_match() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = {}; \
             caches.open('v1').then(function(c) { \
                 return c.put('/a', new Response('AAA', { status: 200 })); \
             }).then(function() { return caches.match('/a'); }) \
              .then(function(resp) { return resp.text(); }).then(function(t) { r.viaStorage = t; }) \
              .then(function() { return caches.open('v1'); }) \
              .then(function(c) { return c.matchAll(); }) \
              .then(function(list) { r.count = list.length; });",
        )
        .expect("e");
        assert_eq!(rt.eval("r.viaStorage").expect("e"), JsValue::String("AAA".into()));
        assert_eq!(rt.eval("r.count").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.92 — StorageEvent + evento storage ----

    #[test]
    fn storage_event_campos_y_es_event() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var e = new StorageEvent('storage', \
                 { key: 'k', oldValue: 'viejo', newValue: 'nuevo', url: 'https://x/' }); \
             var esEvent = e instanceof Event;",
        )
        .expect("e");
        assert_eq!(rt.eval("e.type").expect("e"), JsValue::String("storage".into()));
        assert_eq!(rt.eval("e.key").expect("e"), JsValue::String("k".into()));
        assert_eq!(rt.eval("e.oldValue").expect("e"), JsValue::String("viejo".into()));
        assert_eq!(rt.eval("e.newValue").expect("e"), JsValue::String("nuevo".into()));
        assert_eq!(rt.eval("e.url").expect("e"), JsValue::String("https://x/".into()));
        assert_eq!(rt.eval("esEvent").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn dispatch_storage_entrega_evento_en_window() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "https://example.com/", "b").expect("d");
        rt.eval(
            "var rec = null; \
             addEventListener('storage', function(e) { \
                 rec = e.key + '=' + e.newValue + ':' + (e.storageArea === localStorage) \
                     + ':' + (e instanceof StorageEvent); \
             }); \
             var n = __puriy_dispatch_storage('tema', null, 'oscuro', 'local', 'https://example.com/otra');",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("rec").expect("e"), JsValue::String("tema=oscuro:true:true".into()));
    }

    // ---- Fase 7.93 — Permissions API ----

    #[test]
    fn permissions_query_devuelve_status_y_es_eventtarget() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "__puriy_set_permission('geolocation', 'granted'); \
             var st = null; \
             navigator.permissions.query({ name: 'geolocation' }).then(function(s) { st = s; });",
        )
        .expect("e");
        assert_eq!(rt.eval("st.name").expect("e"), JsValue::String("geolocation".into()));
        assert_eq!(rt.eval("st.state").expect("e"), JsValue::String("granted".into()));
        assert_eq!(rt.eval("st instanceof EventTarget").expect("e"), JsValue::Bool(true));
        // permiso sin setear → 'prompt' (el usuario no decidió).
        rt.eval("navigator.permissions.query({ name: 'camera' }).then(function(s) { globalThis._cam = s; });")
            .expect("e");
        assert_eq!(rt.eval("_cam.state").expect("e"), JsValue::String("prompt".into()));
    }

    #[test]
    fn set_permission_dispara_change_en_status_vivo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var st = null, changed = null; \
             navigator.permissions.query({ name: 'notifications' }).then(function(s) { \
                 st = s; \
                 st.onchange = function() { changed = st.state; }; \
             });",
        )
        .expect("e");
        rt.eval("__puriy_set_permission('notifications', 'denied');").expect("e");
        assert_eq!(rt.eval("changed").expect("e"), JsValue::String("denied".into()));
        assert_eq!(rt.eval("st.state").expect("e"), JsValue::String("denied".into()));
    }

    // ---- Fase 7.94 — Notification API ----

    #[test]
    fn notification_permission_default_y_request() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("Notification.permission").expect("e"), JsValue::String("default".into()));
        rt.eval("var p = null; Notification.requestPermission().then(function(s) { p = s; });")
            .expect("e");
        assert_eq!(rt.eval("p").expect("e"), JsValue::String("default".into()));
        rt.eval("__puriy_set_notification_permission('granted');").expect("e");
        assert_eq!(rt.eval("Notification.permission").expect("e"), JsValue::String("granted".into()));
    }

    #[test]
    fn notification_granted_dispara_show() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "__puriy_set_notification_permission('granted'); \
             var got = null; \
             var n = new Notification('hola', { body: 'cuerpo' }); \
             n.onshow = function() { got = n.title + ':' + n.body; };",
        )
        .expect("e");
        // show se dispara en microtask → ya corrió tras el drain del eval.
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("hola:cuerpo".into()));
    }

    #[test]
    fn notification_sin_permiso_dispara_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = false, shown = false; \
             var n = new Notification('y'); \
             n.onerror = function() { err = true; }; \
             n.onshow = function() { shown = true; };",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("shown").expect("e"), JsValue::Bool(false));
    }

    // ---- Fase 7.95 — navigator.geolocation ----

    #[test]
    fn geolocation_get_current_position_entrega_coords() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var rec = null; \
             navigator.geolocation.getCurrentPosition(function(p) { \
                 rec = p.coords.latitude + ',' + p.coords.longitude + ',' + p.coords.accuracy; \
             }); \
             __puriy_deliver_position(1, { latitude: 10.5, longitude: -66.9, accuracy: 5 });",
        )
        .expect("e");
        assert_eq!(rt.eval("rec").expect("e"), JsValue::String("10.5,-66.9,5".into()));
    }

    #[test]
    fn geolocation_watch_entrega_repetido_y_clear_corta() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var id = navigator.geolocation.watchPosition(function() { n++; }); \
             __puriy_deliver_position(id, { latitude: 1, longitude: 2 }); \
             __puriy_deliver_position(id, { latitude: 3, longitude: 4 }); \
             navigator.geolocation.clearWatch(id); \
             var afterClear = __puriy_deliver_position(id, { latitude: 5, longitude: 6 });",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("afterClear").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn geolocation_error_invoca_callback_de_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var code = null; \
             navigator.geolocation.getCurrentPosition(function() {}, function(e) { \
                 code = e.code + ':' + (e.PERMISSION_DENIED === 1); \
             }); \
             __puriy_deliver_position_error(1, 1, 'denegado');",
        )
        .expect("e");
        assert_eq!(rt.eval("code").expect("e"), JsValue::String("1:true".into()));
    }

    // ---- Fase 7.96 — Clipboard API ----

    #[test]
    fn clipboard_write_text_y_read_text_round_trip() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var done = false; navigator.clipboard.writeText('hola').then(function() { done = true; });")
            .expect("e");
        assert_eq!(rt.eval("done").expect("e"), JsValue::Bool(true));
        rt.eval("var got = null; navigator.clipboard.readText().then(function(t) { got = t; });")
            .expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("hola".into()));
        // writeText publica una mutación clipboard al chrome.
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'clipboard' && d.value === 'writeText:hola'; })")
                .expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn clipboard_set_clipboard_sincroniza_desde_el_chrome() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_clipboard('copiado afuera');").expect("e");
        rt.eval("var got = null; navigator.clipboard.readText().then(function(t) { got = t; });")
            .expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("copiado afuera".into()));
    }

    #[test]
    fn clipboard_set_clipboard_metodo_host_sincroniza_el_buffer() {
        // El chrome empuja el portapapeles del sistema vía el método host
        // `set_clipboard` (no el `__puriy_set_clipboard` crudo). Escapa
        // comillas/saltos sin romper el eval.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_clipboard("línea\ncon 'comillas'").expect("set");
        rt.eval("var got = null; navigator.clipboard.readText().then(function(t) { got = t; });")
            .expect("e");
        assert_eq!(
            rt.eval("got").expect("e"),
            JsValue::String("línea\ncon 'comillas'".into())
        );
    }

    #[test]
    fn clipboard_item_write_y_read_con_blob() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var item = new ClipboardItem({ 'text/plain': new Blob(['desde item'], { type: 'text/plain' }) }); \
             navigator.clipboard.write([item]);",
        )
        .expect("e");
        rt.eval(
            "var leido = null; \
             navigator.clipboard.read() \
                 .then(function(items) { return items[0].getType('text/plain'); }) \
                 .then(function(b) { return b.text(); }) \
                 .then(function(t) { leido = t; });",
        )
        .expect("e");
        assert_eq!(rt.eval("leido").expect("e"), JsValue::String("desde item".into()));
        assert_eq!(
            rt.eval("new ClipboardItem({ 'text/plain': 'x' }).types[0]").expect("e"),
            JsValue::String("text/plain".into())
        );
    }

    // ---- Fase 7.97 — Web Share API ----

    #[test]
    fn share_can_share_evalua_los_datos() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("navigator.canShare({ url: 'https://x' })").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.canShare({ text: 'hola' })").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.canShare({})").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("navigator.canShare()").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn share_publica_mutacion_y_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ok = false; \
             navigator.share({ title: 'T', url: 'https://x' }).then(function() { ok = true; });",
        )
        .expect("e");
        // share publica al chrome y queda pendiente (no resuelve sola).
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(false));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'share'; })").expect("e"),
            JsValue::Bool(true)
        );
        // El chrome resuelve la hoja de share.
        rt.eval("__puriy_share_resolve(1);").expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn share_reject_rechaza_con_dom_exception() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.share({ text: 'hola' }).catch(function(e) { errName = e.name; }); \
             __puriy_share_reject(1, 'AbortError', 'cancelado');",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("AbortError".into()));
    }

    // ---- Fase 7.98 — matchMedia / MediaQueryList ----

    #[test]
    fn match_media_devuelve_mql_con_matches_default_false() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var mql = matchMedia('(prefers-color-scheme: dark)');").expect("e");
        assert_eq!(
            rt.eval("mql.media").expect("e"),
            JsValue::String("(prefers-color-scheme: dark)".into())
        );
        assert_eq!(rt.eval("mql.matches").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("mql instanceof EventTarget").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn set_media_match_flippea_y_dispara_change() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var cambios = []; \
             var mql = matchMedia('(max-width: 600px)'); \
             mql.onchange = function(e) { cambios.push(e.matches); }; \
             mql.addEventListener('change', function(e) { cambios.push('lst:' + e.matches); }); \
             __puriy_set_media_match('(max-width: 600px)', true);",
        )
        .expect("e");
        assert_eq!(rt.eval("mql.matches").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("cambios.join(',')").expect("e"), JsValue::String("true,lst:true".into()));
    }

    #[test]
    fn match_media_add_listener_legacy_recibe_change() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var mql = matchMedia('print'); \
             var fn = function() { n++; }; \
             mql.addListener(fn); \
             __puriy_set_media_match('print', true); \
             mql.removeListener(fn); \
             __puriy_set_media_match('print', false);",
        )
        .expect("e");
        // El listener corrió una sola vez (se quitó antes del segundo cambio).
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn registered_media_queries_enumera_lo_consultado() {
        // Fase 7.174 — el chrome enumera las queries para evaluarlas él mismo.
        let mut rt = JsRuntime::new().expect("rt");
        assert!(rt.registered_media_queries().is_empty());
        rt.eval(
            "matchMedia('(min-width: 600px)'); \
             matchMedia('(orientation: landscape)'); \
             matchMedia('(min-width: 600px)');", // duplicada → dedup
        )
        .expect("e");
        let qs = rt.registered_media_queries();
        assert_eq!(qs.len(), 2, "dedup: {qs:?}");
        assert!(qs.contains(&"(min-width: 600px)".to_string()));
        assert!(qs.contains(&"(orientation: landscape)".to_string()));
    }

    #[test]
    fn set_media_match_host_flippea_y_solo_dispara_si_cambia() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var mql = matchMedia('(min-width: 600px)'); \
             mql.addEventListener('change', function() { n++; });",
        )
        .expect("e");
        // Primer push true → flipea de undefined → dispara.
        rt.set_media_match("(min-width: 600px)", true).expect("set");
        assert_eq!(rt.eval("mql.matches").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
        // Re-empujar el MISMO valor no debe re-disparar change.
        rt.set_media_match("(min-width: 600px)", true).expect("set");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
        // Cambiar a false sí dispara.
        rt.set_media_match("(min-width: 600px)", false).expect("set");
        assert_eq!(rt.eval("mql.matches").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
    }

    // ---- Fase 7.99 — screen / orientation / devicePixelRatio ----

    #[test]
    fn screen_expone_defaults_y_es_instancia_de_screen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("screen.width").expect("e"), JsValue::Number(1280.0));
        assert_eq!(rt.eval("screen.height").expect("e"), JsValue::Number(720.0));
        assert_eq!(rt.eval("screen.colorDepth").expect("e"), JsValue::Number(24.0));
        assert_eq!(rt.eval("devicePixelRatio").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("screen instanceof Screen").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("screen.orientation instanceof EventTarget").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn set_screen_y_device_pixel_ratio_actualizan_los_getters() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_screen({ width: 390, height: 844, availHeight: 800 });").expect("e");
        rt.eval("__puriy_set_device_pixel_ratio(3);").expect("e");
        assert_eq!(rt.eval("screen.width").expect("e"), JsValue::Number(390.0));
        assert_eq!(rt.eval("screen.availHeight").expect("e"), JsValue::Number(800.0));
        // height intacto (no venía en el patch).
        assert_eq!(rt.eval("screen.height").expect("e"), JsValue::Number(844.0));
        assert_eq!(rt.eval("devicePixelRatio").expect("e"), JsValue::Number(3.0));
    }

    #[test]
    fn set_device_pixel_ratio_metodo_host_actualiza_el_getter() {
        // Fase 7.173 — el chrome alimenta el scale_factor de winit por aquí.
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("devicePixelRatio").expect("e"), JsValue::Number(1.0));
        rt.set_device_pixel_ratio(2.0).expect("set dpr");
        assert_eq!(rt.eval("devicePixelRatio").expect("e"), JsValue::Number(2.0));
        // Valores no-finitos o <= 0 son ignorados (spec: dpr > 0 siempre).
        rt.set_device_pixel_ratio(f64::NAN).expect("nan no-op");
        rt.set_device_pixel_ratio(0.0).expect("cero no-op");
        rt.set_device_pixel_ratio(-1.0).expect("neg no-op");
        assert_eq!(rt.eval("devicePixelRatio").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn set_orientation_flippea_type_angle_y_dispara_change() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var got = []; \
             screen.orientation.onchange = function() { got.push(screen.orientation.type); }; \
             screen.orientation.addEventListener('change', function(e) { got.push('lst:' + e.type); }); \
             __puriy_set_orientation('portrait-primary', 90);",
        )
        .expect("e");
        assert_eq!(
            rt.eval("screen.orientation.type").expect("e"),
            JsValue::String("portrait-primary".into())
        );
        assert_eq!(rt.eval("screen.orientation.angle").expect("e"), JsValue::Number(90.0));
        assert_eq!(
            rt.eval("got.join(',')").expect("e"),
            JsValue::String("portrait-primary,lst:change".into())
        );
    }

    #[test]
    fn orientation_lock_rechaza_con_not_supported() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             screen.orientation.lock('portrait').catch(function(e) { errName = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotSupportedError".into()));
    }

    // ---- Fase 7.100 — navigator.serviceWorker ----

    #[test]
    fn service_worker_existe_y_controller_es_null() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("'serviceWorker' in navigator").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(rt.eval("navigator.serviceWorker.controller").expect("e"), JsValue::Null);
        assert_eq!(
            rt.eval("typeof navigator.serviceWorker.register").expect("e"),
            JsValue::String("function".into())
        );
    }

    #[test]
    fn service_worker_register_publica_mutacion_y_resuelve_registration() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var scope = null; \
             navigator.serviceWorker.register('/sw.js', { scope: '/app/' }) \
                 .then(function(reg) { scope = reg.scope; });",
        )
        .expect("e");
        // Publicó la mutación serviceworker-register al chrome.
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'serviceworker-register'; })")
                .expect("e"),
            JsValue::Bool(true)
        );
        // El chrome resuelve el registro pendiente (id=1).
        rt.eval("__puriy_serviceworker_resolve(1, '/app/');").expect("e");
        assert_eq!(rt.eval("scope").expect("e"), JsValue::String("/app/".into()));
    }

    #[test]
    fn service_worker_register_reject_rechaza_con_dom_exception() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.serviceWorker.register('/sw.js').catch(function(e) { errName = e.name; }); \
             __puriy_serviceworker_reject(1, 'SecurityError', 'no');",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("SecurityError".into()));
    }

    #[test]
    fn service_worker_get_registrations_resuelve_vacio() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = -1; \
             navigator.serviceWorker.getRegistrations().then(function(r) { n = r.length; });",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.101 — navigator.mediaDevices ----

    #[test]
    fn media_devices_existe_y_get_user_media_rechaza_por_defecto() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof navigator.mediaDevices.getUserMedia").expect("e"),
            JsValue::String("function".into())
        );
        rt.eval(
            "var errName = null; \
             navigator.mediaDevices.getUserMedia({ video: true }).catch(function(e) { errName = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotAllowedError".into()));
    }

    #[test]
    fn media_devices_get_user_media_sin_constraints_es_type_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ok = false; \
             navigator.mediaDevices.getUserMedia({}).catch(function(e) { ok = (e instanceof TypeError); });",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn media_devices_permiso_concedido_resuelve_stream() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_media_devices_permission(true);").expect("e");
        rt.eval(
            "var active = null; \
             navigator.mediaDevices.getUserMedia({ audio: true }) \
                 .then(function(s) { active = (s instanceof MediaStream) && s.active; });",
        )
        .expect("e");
        assert_eq!(rt.eval("active").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn media_devices_enumerate_y_devicechange() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = -1; var cambios = 0; \
             navigator.mediaDevices.ondevicechange = function() { cambios++; }; \
             __puriy_set_media_devices([{ kind: 'audioinput', deviceId: 'mic1' }]); \
             navigator.mediaDevices.enumerateDevices().then(function(d) { n = d.length; });",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("cambios").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.102 — navigator.getBattery / BatteryManager ----

    #[test]
    fn get_battery_resuelve_singleton_con_defaults() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var b1 = null; var b2 = null; \
             navigator.getBattery().then(function(b) { b1 = b; }); \
             navigator.getBattery().then(function(b) { b2 = b; });",
        )
        .expect("e");
        assert_eq!(rt.eval("b1 === b2").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("b1.charging").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("b1.level").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("b1 instanceof BatteryManager").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("b1 instanceof EventTarget").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn set_battery_flippea_y_dispara_los_change_correspondientes() {
        let mut rt = JsRuntime::new().expect("rt");
        // El callback de getBattery() corre como microtask al final del eval;
        // attach los handlers ANTES de setear (eval separado) o llegan tarde.
        rt.eval(
            "var got = []; var b = null; \
             navigator.getBattery().then(function(x) { b = x; \
                 b.onlevelchange = function() { got.push('level:' + b.level); }; \
                 b.onchargingchange = function() { got.push('charging:' + b.charging); }; \
             });",
        )
        .expect("e");
        rt.eval("__puriy_set_battery({ level: 0.5, charging: false });").expect("e");
        assert_eq!(rt.eval("b.level").expect("e"), JsValue::Number(0.5));
        assert_eq!(rt.eval("b.charging").expect("e"), JsValue::Bool(false));
        assert_eq!(
            rt.eval("got.join(',')").expect("e"),
            JsValue::String("charging:false,level:0.5".into())
        );
    }

    #[test]
    fn set_battery_no_dispara_si_el_valor_no_cambia() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; var b = null; \
             navigator.getBattery().then(function(x) { b = x; \
                 b.onlevelchange = function() { n++; }; });",
        )
        .expect("e");
        rt.eval("__puriy_set_battery({ level: 1.0 });").expect("e");
        // level ya era 1.0 → sin evento.
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.103 — navigator.wakeLock ----

    #[test]
    fn wake_lock_request_resuelve_sentinel_y_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var s = null; \
             navigator.wakeLock.request('screen').then(function(x) { s = x; });",
        )
        .expect("e");
        assert_eq!(rt.eval("s.type").expect("e"), JsValue::String("screen".into()));
        assert_eq!(rt.eval("s.released").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("s instanceof WakeLockSentinel").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'wakelock-request'; })")
                .expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn wake_lock_release_marca_released_y_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var liberado = false; var s = null; \
             navigator.wakeLock.request().then(function(x) { s = x; \
                 s.addEventListener('release', function() { liberado = true; }); \
                 s.release(); });",
        )
        .expect("e");
        assert_eq!(rt.eval("s.released").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("liberado").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d) { return d.kind === 'wakelock-release'; })")
                .expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn wake_lock_denegado_rechaza_con_not_allowed() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "__puriy_set_wakelock_permission(false); \
             var errName = null; \
             navigator.wakeLock.request('screen').catch(function(e) { errName = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotAllowedError".into()));
    }

    // ---- Fase 7.104 — navigator.storage (StorageManager) ----

    #[test]
    fn storage_estimate_devuelve_defaults() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var usage = -1; var quota = -1; \
             navigator.storage.estimate().then(function(e) { usage = e.usage; quota = e.quota; });",
        )
        .expect("e");
        assert_eq!(rt.eval("usage").expect("e"), JsValue::Number(0.0));
        assert_eq!(
            rt.eval("quota").expect("e"),
            JsValue::Number(2.0 * 1024.0 * 1024.0 * 1024.0)
        );
    }

    #[test]
    fn set_storage_estimate_y_persisted_actualizan() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_storage_estimate({ usage: 1000, quota: 5000 });").expect("e");
        rt.eval("__puriy_set_storage_persisted(true);").expect("e");
        rt.eval(
            "var u = -1; var p = null; \
             navigator.storage.estimate().then(function(e) { u = e.usage; }); \
             navigator.storage.persisted().then(function(x) { p = x; });",
        )
        .expect("e");
        assert_eq!(rt.eval("u").expect("e"), JsValue::Number(1000.0));
        assert_eq!(rt.eval("p").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn storage_get_directory_rechaza_security_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.storage.getDirectory().catch(function(e) { errName = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("SecurityError".into()));
    }

    // ---- Fase 7.105 — navigator.locks (Web Locks API) ----
