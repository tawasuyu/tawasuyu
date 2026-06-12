//! Tests de Fullscreen, PointerLock, WebBluetooth, FileSystemAccess, WebAnimations, WebAuthn, WebTransport, PushAPI, BackgroundSync, Sensors, NFC, Presentation, TrustedTypes, Reporting, ComputePressure, Navigation, ViewTransitions, CookieStore, IndexedDB, WebRTC, Workers.
    use super::*;

    // ---- Fase 7.123 — Fullscreen API ----

    #[test]
    fn fullscreen_api_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof document.exitFullscreen").expect("e"),
            JsValue::String("function".into())
        );
        assert_eq!(rt.eval("document.fullscreenEnabled").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("document.fullscreenElement").expect("e"), JsValue::Null);
    }

    #[test]
    fn fullscreen_request_publica_mutacion_y_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var ok = false; el.requestFullscreen().then(function(){ ok = true; }); \
             __puriy_fullscreen_resolve('el1');",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("document.fullscreenElement && document.fullscreenElement._id").expect("e"),
            JsValue::String("el1".into())
        );
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'fullscreen-request'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn fullscreen_exit_limpia_y_dispara_change() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var changes = 0; document.onfullscreenchange = function(){ changes++; }; \
             el.requestFullscreen(); __puriy_fullscreen_resolve('el1'); \
             document.exitFullscreen();",
        )
        .expect("e");
        assert_eq!(rt.eval("document.fullscreenElement").expect("e"), JsValue::Null);
        // Un change al entrar + uno al salir.
        assert_eq!(rt.eval("changes").expect("e"), JsValue::Number(2.0));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'fullscreen-exit'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn fullscreen_reject_dispara_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var errName = null; var errs = 0; \
             document.onfullscreenerror = function(){ errs++; }; \
             el.requestFullscreen().catch(function(e){ errName = e.name; }); \
             __puriy_fullscreen_reject('el1');",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("TypeError".into()));
        assert_eq!(rt.eval("errs").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("document.fullscreenElement").expect("e"), JsValue::Null);
    }

    // ---- Fase 7.124 — Pointer Lock API ----

    #[test]
    fn pointerlock_api_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof document.exitPointerLock").expect("e"),
            JsValue::String("function".into())
        );
        assert_eq!(rt.eval("document.pointerLockElement").expect("e"), JsValue::Null);
    }

    #[test]
    fn pointerlock_request_resuelve_y_setea_element() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var ok = false; el.requestPointerLock().then(function(){ ok = true; }); \
             __puriy_pointerlock_resolve('el1');",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("document.pointerLockElement && document.pointerLockElement._id").expect("e"),
            JsValue::String("el1".into())
        );
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'pointerlock-request'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn pointerlock_exit_limpia() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var changes = 0; document.onpointerlockchange = function(){ changes++; }; \
             el.requestPointerLock(); __puriy_pointerlock_resolve('el1'); \
             document.exitPointerLock();",
        )
        .expect("e");
        assert_eq!(rt.eval("document.pointerLockElement").expect("e"), JsValue::Null);
        assert_eq!(rt.eval("changes").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn pointerlock_reject_dispara_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var errName = null; var errs = 0; \
             document.onpointerlockerror = function(){ errs++; }; \
             el.requestPointerLock().catch(function(e){ errName = e.name; }); \
             __puriy_pointerlock_reject('el1');",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotSupportedError".into()));
        assert_eq!(rt.eval("errs").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.125 — Web Bluetooth API ----

    #[test]
    fn bluetooth_namespace_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof navigator.bluetooth.requestDevice").expect("e"),
            JsValue::String("function".into())
        );
    }

    #[test]
    fn bluetooth_request_device_rechaza_sin_filtros() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.bluetooth.requestDevice({}).catch(function(e){ errName = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("TypeError".into()));
    }

    #[test]
    fn bluetooth_request_device_resuelve_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var nombre = null; \
             navigator.bluetooth.requestDevice({ acceptAllDevices: true }).then(function(d){ nombre = d.name; }); \
             __puriy_bluetooth_resolve({ id: 'w1', name: 'Reloj' });",
        )
        .expect("e");
        assert_eq!(rt.eval("nombre").expect("e"), JsValue::String("Reloj".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'bluetooth-request'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn bluetooth_request_device_rechaza_notfound() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.bluetooth.requestDevice({ acceptAllDevices: true }).catch(function(e){ errName = e.name; }); \
             __puriy_bluetooth_reject();",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotFoundError".into()));
    }

    #[test]
    fn bluetooth_gatt_connect_resuelve_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var conectado = false; \
             navigator.bluetooth.requestDevice({ acceptAllDevices: true }).then(function(d){ \
                d.gatt.connect().then(function(srv){ conectado = srv.connected; }); \
                __puriy_bluetooth_gatt_resolve(d.id); \
             }); \
             __puriy_bluetooth_resolve({ id: 'w1', name: 'Reloj' });",
        )
        .expect("e");
        assert_eq!(rt.eval("conectado").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn bluetooth_get_availability_refleja_estado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var a = null; navigator.bluetooth.getAvailability().then(function(v){ a = v; });")
            .expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::Bool(true));
        rt.eval(
            "__puriy_set_bluetooth_availability(false); \
             var b = null; navigator.bluetooth.getAvailability().then(function(v){ b = v; });",
        )
        .expect("e");
        assert_eq!(rt.eval("b").expect("e"), JsValue::Bool(false));
    }

    // ---- Fase 7.126 — File System Access API ----

    #[test]
    fn filesystem_pickers_existen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof showOpenFilePicker").expect("e"),
            JsValue::String("function".into())
        );
        assert_eq!(
            rt.eval("typeof showSaveFilePicker + ',' + typeof showDirectoryPicker").expect("e"),
            JsValue::String("function,function".into())
        );
    }

    #[test]
    fn filesystem_open_picker_resuelve_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var nombre = null, kind = null; \
             showOpenFilePicker().then(function(list){ nombre = list[0].name; kind = list[0].kind; }); \
             var id = __puriy_fs_next_id - 1; \
             __puriy_fs_open_resolve(id, [{ name: 'notas.txt', content: 'hola' }]);",
        )
        .expect("e");
        assert_eq!(rt.eval("nombre").expect("e"), JsValue::String("notas.txt".into()));
        assert_eq!(rt.eval("kind").expect("e"), JsValue::String("file".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'fs-open-picker'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn filesystem_open_picker_cancelado_rechaza_abort() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             showOpenFilePicker().catch(function(e){ errName = e.name; }); \
             var id = __puriy_fs_next_id - 1; \
             __puriy_fs_reject(id);",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("AbortError".into()));
    }

    #[test]
    fn filesystem_writable_buffer_round_trip() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var leido = null; \
             showSaveFilePicker().then(function(h){ \
                h.createWritable().then(function(w){ \
                    w.write('contenido'); \
                    w.close().then(function(){ \
                        h.getFile().then(function(f){ leido = h._content; }); \
                    }); \
                }); \
             }); \
             var id = __puriy_fs_next_id - 1; \
             __puriy_fs_save_resolve(id, { name: 'out.txt' });",
        )
        .expect("e");
        assert_eq!(rt.eval("leido").expect("e"), JsValue::String("contenido".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'fs-write'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn filesystem_directory_get_file_handle_create() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var nombre = null; \
             showDirectoryPicker().then(function(dir){ \
                dir.getFileHandle('nuevo.txt', { create: true }).then(function(h){ nombre = h.name; }); \
             }); \
             var id = __puriy_fs_next_id - 1; \
             __puriy_fs_directory_resolve(id, { name: 'docs' });",
        )
        .expect("e");
        assert_eq!(rt.eval("nombre").expect("e"), JsValue::String("nuevo.txt".into()));
    }

    // ---- Fase 7.127 — Web Animations API ----

    #[test]
    fn animations_animate_devuelve_animation_running() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval("var a = el.animate([{ opacity: 0 }, { opacity: 1 }], 1000);").expect("e");
        assert_eq!(
            rt.eval("a instanceof Animation").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(rt.eval("a.playState").expect("e"), JsValue::String("running".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'animate'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn animations_finish_resuelve_finished_y_dispara_onfinish() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var done = false, fired = 0; \
             var a = el.animate([], 1000); \
             a.onfinish = function(){ fired++; }; \
             a.finished.then(function(){ done = true; }); \
             a.finish();",
        )
        .expect("e");
        assert_eq!(rt.eval("a.playState").expect("e"), JsValue::String("finished".into()));
        assert_eq!(rt.eval("done").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("fired").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn animations_pause_cambia_play_state() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval("var a = el.animate([], 1000); a.pause();").expect("e");
        assert_eq!(rt.eval("a.playState").expect("e"), JsValue::String("paused".into()));
    }

    #[test]
    fn animations_cancel_rechaza_finished_y_dispara_oncancel() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval(
            "var errName = null, fired = 0; \
             var a = el.animate([], 1000); \
             a.oncancel = function(){ fired++; }; \
             a.finished.catch(function(e){ errName = e.name; }); \
             a.cancel();",
        )
        .expect("e");
        assert_eq!(rt.eval("a.playState").expect("e"), JsValue::String("idle".into()));
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("AbortError".into()));
        assert_eq!(rt.eval("fired").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.128 — Web Authentication (WebAuthn) ----

    #[test]
    fn webauthn_public_key_credential_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof PublicKeyCredential").expect("e"),
            JsValue::String("function".into())
        );
        assert_eq!(
            rt.eval("typeof PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable").expect("e"),
            JsValue::String("function".into())
        );
    }

    #[test]
    fn webauthn_create_publickey_resuelve_credential() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var tipo = null, attest = null; \
             navigator.credentials.create({ publicKey: { challenge: 'x' } }).then(function(c){ \
                tipo = c.type; attest = c.response.attestationObject; \
             }); \
             var id = __puriy_webauthn_next_id - 1; \
             __puriy_webauthn_resolve(id, { id: 'cred1', response: { attestationObject: 'AAA', clientDataJSON: '{}' } });",
        )
        .expect("e");
        assert_eq!(rt.eval("tipo").expect("e"), JsValue::String("public-key".into()));
        assert_eq!(rt.eval("attest").expect("e"), JsValue::String("AAA".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'webauthn-create'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn webauthn_get_publickey_resuelve_assertion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var sig = null; \
             navigator.credentials.get({ publicKey: { challenge: 'y' } }).then(function(c){ \
                sig = c.response.signature; \
             }); \
             var id = __puriy_webauthn_next_id - 1; \
             __puriy_webauthn_resolve(id, { id: 'cred1', response: { signature: 'SIG', authenticatorData: 'AD' } });",
        )
        .expect("e");
        assert_eq!(rt.eval("sig").expect("e"), JsValue::String("SIG".into()));
    }

    #[test]
    fn webauthn_create_cancelado_rechaza_not_allowed() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             navigator.credentials.create({ publicKey: {} }).catch(function(e){ errName = e.name; }); \
             var id = __puriy_webauthn_next_id - 1; \
             __puriy_webauthn_reject(id);",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotAllowedError".into()));
    }

    #[test]
    fn webauthn_uvpaa_refleja_estado_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var a = null; \
             PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable().then(function(v){ a = v; });",
        )
        .expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::Bool(false));
        rt.eval(
            "__puriy_set_uvpaa(true); var b = null; \
             PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable().then(function(v){ b = v; });",
        )
        .expect("e");
        assert_eq!(rt.eval("b").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webauthn_sin_publickey_delega_en_credentials_original() {
        let mut rt = JsRuntime::new().expect("rt");
        // create() sin publicKey publica la mutación 'credentials' (Fase 7.110),
        // no 'webauthn-create'.
        rt.eval("navigator.credentials.create({ password: { id: 'a' } });").expect("e");
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'credentials'; })").expect("e"),
            JsValue::Bool(true)
        );
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'webauthn-create'; })").expect("e"),
            JsValue::Bool(false)
        );
    }

    // ---- Fase 7.129 — WebTransport ----

    #[test]
    fn transport_constructor_publica_connect_y_ready() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var listo = false; \
             var wt = new WebTransport('https://example.com:443/echo'); \
             wt.ready.then(function(){ listo = true; }); \
             __puriy_wt_dispatch(wt._id, 'ready');",
        )
        .expect("e");
        assert_eq!(rt.eval("listo").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'webtransport'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn transport_close_resuelve_closed() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var code = null; \
             var wt = new WebTransport('https://example.com/x'); \
             wt.closed.then(function(info){ code = info.closeCode; }); \
             wt.close({ closeCode: 7, reason: 'fin' });",
        )
        .expect("e");
        assert_eq!(rt.eval("code").expect("e"), JsValue::Number(7.0));
    }

    #[test]
    fn transport_dispatch_close_error_rechaza_ready() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             var wt = new WebTransport('https://example.com/x'); \
             wt.ready.catch(function(e){ errName = e.name; }); \
             __puriy_wt_dispatch(wt._id, 'close', 0, '', '1');",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NetworkError".into()));
    }

    #[test]
    fn transport_datagram_writer_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var wt = new WebTransport('https://example.com/x'); \
             var w = wt.datagrams.writable.getWriter(); \
             w.write('hola');",
        )
        .expect("e");
        assert_eq!(
            rt.eval(
                "__puriy_dirty.some(function(d){ \
                   return d.kind === 'webtransport' && d.value.indexOf('datagram') !== -1; })"
            )
            .expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn transport_create_bidirectional_stream() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var tieneReadable = false, tieneWritable = false; \
             var wt = new WebTransport('https://example.com/x'); \
             wt.createBidirectionalStream().then(function(s){ \
                tieneReadable = (s.readable != null); \
                tieneWritable = (typeof s.writable.getWriter === 'function'); \
             });",
        )
        .expect("e");
        assert_eq!(rt.eval("tieneReadable").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("tieneWritable").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.130 — Push API ----

    #[test]
    fn push_manager_existe_en_registration() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        assert_eq!(
            rt.eval("typeof reg.pushManager.subscribe").expect("e"),
            JsValue::String("function".into())
        );
        assert_eq!(
            rt.eval("reg.pushManager instanceof PushManager").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn push_subscribe_resuelve_subscription() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval(
            "var endpoint = null, clave = null; \
             reg.pushManager.subscribe({ userVisibleOnly: true }).then(function(sub){ \
                endpoint = sub.endpoint; clave = sub.getKey('p256dh'); \
             }); \
             __puriy_push_resolve(__puriy_push_next_id - 1, \
                { endpoint: 'https://push.example/abc', keys: { p256dh: 'KEY' } });",
        )
        .expect("e");
        assert_eq!(
            rt.eval("endpoint").expect("e"),
            JsValue::String("https://push.example/abc".into())
        );
        assert_eq!(rt.eval("clave").expect("e"), JsValue::String("KEY".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'push-subscribe'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn push_subscribe_cancelado_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval(
            "var errName = null; \
             reg.pushManager.subscribe({}).catch(function(e){ errName = e.name; }); \
             __puriy_push_reject(__puriy_push_next_id - 1);",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotAllowedError".into()));
    }

    #[test]
    fn push_get_subscription_devuelve_actual() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval(
            "var same = false; \
             reg.pushManager.subscribe({}).then(function(sub){ \
                reg.pushManager.getSubscription().then(function(s){ same = (s === sub); }); \
             }); \
             __puriy_push_resolve(__puriy_push_next_id - 1, { endpoint: 'https://p/x' });",
        )
        .expect("e");
        assert_eq!(rt.eval("same").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn push_permission_state_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval("var a = null; reg.pushManager.permissionState().then(function(v){ a = v; });")
            .expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::String("prompt".into()));
        rt.eval(
            "__puriy_set_push_permission('granted'); var b = null; \
             reg.pushManager.permissionState().then(function(v){ b = v; });",
        )
        .expect("e");
        assert_eq!(rt.eval("b").expect("e"), JsValue::String("granted".into()));
    }

    // ---- Fase 7.131 — Background Sync + Periodic Background Sync ----

    #[test]
    fn sync_manager_existe_en_registration() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        assert_eq!(
            rt.eval("typeof reg.sync.register").expect("e"),
            JsValue::String("function".into())
        );
        assert_eq!(rt.eval("reg.sync instanceof SyncManager").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn sync_register_publica_y_get_tags() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval(
            "var tags = null; \
             reg.sync.register('subir-fotos').then(function(){ \
                reg.sync.getTags().then(function(t){ tags = t.join(','); }); \
             });",
        )
        .expect("e");
        assert_eq!(rt.eval("tags").expect("e"), JsValue::String("subir-fotos".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'sync-register'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn periodicsync_register_y_unregister() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval(
            "var antes = null, despues = null; \
             reg.periodicSync.register('feed', { minInterval: 86400000 }).then(function(){ \
                reg.periodicSync.getTags().then(function(t){ antes = t.join(','); }); \
                reg.periodicSync.unregister('feed').then(function(){ \
                    reg.periodicSync.getTags().then(function(t){ despues = t.length; }); \
                }); \
             });",
        )
        .expect("e");
        assert_eq!(rt.eval("antes").expect("e"), JsValue::String("feed".into()));
        assert_eq!(rt.eval("despues").expect("e"), JsValue::Number(0.0));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'periodicsync-register'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    // ---- Fase 7.132 — Generic Sensor API ----

    #[test]
    fn sensor_clases_existen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof Accelerometer").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof Gyroscope").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof AmbientLightSensor").expect("e"), JsValue::String("function".into()));
        assert_eq!(
            rt.eval("(new Accelerometer()) instanceof Sensor").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn sensor_start_activa_y_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var activado = false; \
             var s = new Accelerometer({ frequency: 60 }); \
             s.onactivate = function(){ activado = true; }; \
             s.start();",
        )
        .expect("e");
        assert_eq!(rt.eval("s.activated").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("activado").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'sensor-start' && d.value === 'accelerometer'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn sensor_reading_actualiza_campos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var leido = false; \
             var s = new Accelerometer(); s.start(); \
             s.onreading = function(){ leido = true; }; \
             __puriy_sensor_reading('accelerometer', { x: 1, y: 2, z: 3, timestamp: 99 });",
        )
        .expect("e");
        assert_eq!(rt.eval("leido").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("s.x").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("s.z").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("s.hasReading").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("s.timestamp").expect("e"), JsValue::Number(99.0));
    }

    #[test]
    fn sensor_stop_deja_de_recibir() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var s = new Gyroscope(); s.start(); \
             s.onreading = function(){ n++; }; \
             __puriy_sensor_reading('gyroscope', { x: 1, y: 0, z: 0 }); \
             s.stop(); \
             __puriy_sensor_reading('gyroscope', { x: 9, y: 0, z: 0 });",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("s.x").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("s.activated").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn sensor_ambient_light_y_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; \
             var s = new AmbientLightSensor(); s.start(); \
             s.onerror = function(ev){ err = ev.error.name; }; \
             __puriy_sensor_reading('ambient-light', { illuminance: 320 }); \
             __puriy_sensor_error('ambient-light', 'NotReadableError', 'sin acceso');",
        )
        .expect("e");
        assert_eq!(rt.eval("s.illuminance").expect("e"), JsValue::Number(320.0));
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("NotReadableError".into()));
    }

    // ---- Fase 7.133 — Web NFC ----

    #[test]
    fn nfc_clases_existen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof NDEFReader").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof NDEFMessage").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof NDEFRecord").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn nfc_scan_publica_y_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ok = false; \
             var r = new NDEFReader(); \
             r.scan().then(function(){ ok = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'nfc-scan'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn nfc_reading_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var serie = null, tipo = null; \
             var r = new NDEFReader(); r.scan(); \
             r.onreading = function(ev){ serie = ev.serialNumber; tipo = ev.message.records[0].recordType; }; \
             __puriy_nfc_reading('04:A2:3F', [{ recordType: 'url', data: 'https://x' }]);",
        )
        .expect("e");
        assert_eq!(rt.eval("serie").expect("e"), JsValue::String("04:A2:3F".into()));
        assert_eq!(rt.eval("tipo").expect("e"), JsValue::String("url".into()));
    }

    #[test]
    fn nfc_write_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ok = false; \
             var r = new NDEFReader(); \
             r.write({ records: [{ recordType: 'text', data: 'hola' }] }).then(function(){ ok = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'nfc-write'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    // ---- Fase 7.134 — Presentation API ----

    #[test]
    fn presentation_request_y_navigator() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(
            rt.eval("typeof PresentationRequest").expect("e"),
            JsValue::String("function".into())
        );
        assert_eq!(
            rt.eval("typeof navigator.presentation").expect("e"),
            JsValue::String("object".into())
        );
        assert_eq!(
            rt.eval("(new PresentationRequest('https://recv/x')).urls[0]").expect("e"),
            JsValue::String("https://recv/x".into())
        );
    }

    #[test]
    fn presentation_start_resuelve_connection() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var estado = null, url = null; \
             var pr = new PresentationRequest('https://recv/slides'); \
             pr.start().then(function(c){ estado = c.state; url = c.url; }); \
             __puriy_presentation_resolve(__puriy_presentation_next_id - 1, { id: 'c1', url: 'https://recv/slides' });",
        )
        .expect("e");
        assert_eq!(rt.eval("estado").expect("e"), JsValue::String("connected".into()));
        assert_eq!(rt.eval("url").expect("e"), JsValue::String("https://recv/slides".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'presentation-start'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn presentation_start_cancelado_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             var pr = new PresentationRequest('https://recv/x'); \
             pr.start().catch(function(e){ errName = e.name; }); \
             __puriy_presentation_reject(__puriy_presentation_next_id - 1);",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotAllowedError".into()));
    }

    #[test]
    fn presentation_send_y_message() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var recibido = null; \
             var pr = new PresentationRequest('https://recv/x'); \
             pr.start().then(function(c){ \
                c.onmessage = function(ev){ recibido = ev.data; }; \
                c.send('ping'); \
             }); \
             __puriy_presentation_resolve(__puriy_presentation_next_id - 1, { id: 'conn-x', url: 'https://recv/x' });",
        )
        .expect("e");
        rt.eval("__puriy_presentation_message('conn-x', 'pong');").expect("e");
        assert_eq!(rt.eval("recibido").expect("e"), JsValue::String("pong".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'presentation-send'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn presentation_availability_refleja_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var a = null; \
             var pr = new PresentationRequest('https://recv/x'); \
             pr.getAvailability().then(function(av){ a = av.value; });",
        )
        .expect("e");
        assert_eq!(rt.eval("a").expect("e"), JsValue::Bool(false));
        rt.eval(
            "__puriy_set_presentation_availability(true); var b = null; \
             pr.getAvailability().then(function(av){ b = av.value; });",
        )
        .expect("e");
        assert_eq!(rt.eval("b").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.135 — Trusted Types ----

    #[test]
    fn trusted_types_factory_y_clases_existen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof trustedTypes").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("typeof trustedTypes.createPolicy").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof TrustedHTML").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("trustedTypes.defaultPolicy").expect("e"), JsValue::Null);
    }

    #[test]
    fn trusted_types_policy_envuelve_y_es_reconocida() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var p = trustedTypes.createPolicy('mi', { createHTML: function(s){ return s.replace('<', '&lt;'); } }); \
             var h = p.createHTML('<b>x');",
        )
        .expect("e");
        assert_eq!(rt.eval("h instanceof TrustedHTML").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("h.toString()").expect("e"), JsValue::String("&lt;b>x".into()));
        assert_eq!(rt.eval("trustedTypes.isHTML(h)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("trustedTypes.isScript(h)").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn trusted_types_metodo_faltante_tira_type_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; \
             var p = trustedTypes.createPolicy('solohtml', { createHTML: function(s){ return s; } }); \
             try { p.createScript('alert(1)'); } catch (e) { err = e.constructor.name; }",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("TypeError".into()));
    }

    #[test]
    fn trusted_types_wrapper_no_construible_y_default_policy() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; try { new TrustedHTML(); } catch (e) { err = e.constructor.name; } \
             trustedTypes.createPolicy('default', { createHTML: function(s){ return s; } });",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("TypeError".into()));
        assert_eq!(rt.eval("trustedTypes.defaultPolicy.name").expect("e"), JsValue::String("default".into()));
    }

    // ---- Fase 7.136 — Reporting API ----

    #[test]
    fn reporting_observer_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof ReportingObserver").expect("e"), JsValue::String("function".into()));
        assert_eq!(
            rt.eval("typeof (new ReportingObserver(function(){})).observe").expect("e"),
            JsValue::String("function".into())
        );
    }

    #[test]
    fn reporting_observe_recibe_reportes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var got = null; \
             var o = new ReportingObserver(function(recs){ got = recs[0]; }); \
             o.observe(); \
             __puriy_queue_report({ type: 'deprecation', url: 'https://x', body: { id: 'foo' } });",
        )
        .expect("e");
        assert_eq!(rt.eval("got.type").expect("e"), JsValue::String("deprecation".into()));
        assert_eq!(rt.eval("got.body.id").expect("e"), JsValue::String("foo".into()));
    }

    #[test]
    fn reporting_types_filtra() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var o = new ReportingObserver(function(recs){ n += recs.length; }, { types: ['deprecation'] }); \
             o.observe(); \
             __puriy_queue_report({ type: 'intervention' }); \
             __puriy_queue_report({ type: 'deprecation' });",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn reporting_buffered_reentrega_previos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "__puriy_queue_report({ type: 'deprecation', url: 'https://prev' }); \
             var got = null; \
             var o = new ReportingObserver(function(recs){ got = recs[0]; }, { buffered: true }); \
             o.observe();",
        )
        .expect("e");
        assert_eq!(rt.eval("got.url").expect("e"), JsValue::String("https://prev".into()));
    }

    #[test]
    fn reporting_disconnect_detiene() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var o = new ReportingObserver(function(recs){ n += recs.length; }); \
             o.observe(); o.disconnect(); \
             __puriy_queue_report({ type: 'deprecation' });",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.137 — Compute Pressure API ----

    #[test]
    fn pressure_observer_existe_y_known_sources() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof PressureObserver").expect("e"), JsValue::String("function".into()));
        assert_eq!(
            rt.eval("PressureObserver.knownSources.indexOf('cpu') >= 0").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn pressure_observe_resuelve_y_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ok = false; \
             var o = new PressureObserver(function(){}); \
             o.observe('cpu').then(function(){ ok = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'pressure-observe' && d.value === 'cpu'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn pressure_observe_fuente_desconocida_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var errName = null; \
             var o = new PressureObserver(function(){}); \
             o.observe('gpu').catch(function(e){ errName = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("errName").expect("e"), JsValue::String("NotSupportedError".into()));
    }

    #[test]
    fn pressure_sample_invoca_callback() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var estado = null; \
             var o = new PressureObserver(function(recs){ estado = recs[0].state; }); \
             o.observe('cpu'); \
             __puriy_pressure_sample('cpu', 'serious');",
        )
        .expect("e");
        assert_eq!(rt.eval("estado").expect("e"), JsValue::String("serious".into()));
    }

    #[test]
    fn pressure_unobserve_detiene() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var o = new PressureObserver(function(recs){ n += recs.length; }); \
             o.observe('cpu'); o.unobserve('cpu'); \
             __puriy_pressure_sample('cpu', 'fair');",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.138 — Navigation API ----

    #[test]
    fn navigation_existe_y_current_entry() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof navigation").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("typeof navigation.navigate").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("navigation.entries().length").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("typeof navigation.currentEntry.url").expect("e"), JsValue::String("string".into()));
        assert_eq!(rt.eval("navigation.canGoBack").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn navigation_navigate_agrega_entry_y_actualiza_current() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("navigation.navigate('https://ex/a');").expect("e");
        assert_eq!(rt.eval("navigation.entries().length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("navigation.currentEntry.url").expect("e"), JsValue::String("https://ex/a".into()));
        assert_eq!(rt.eval("navigation.canGoBack").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn navigation_navigate_dispara_navigate_event() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var destino = null; \
             navigation.addEventListener('navigate', function(e){ destino = e.destination.url; }); \
             navigation.navigate('https://ex/b');",
        )
        .expect("e");
        assert_eq!(rt.eval("destino").expect("e"), JsValue::String("https://ex/b".into()));
    }

    #[test]
    fn navigation_intercept_resuelve_finished() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var fin = false, corrio = false; \
             navigation.addEventListener('navigate', function(e){ \
                 e.intercept({ handler: function(){ corrio = true; return Promise.resolve(); } }); \
             }); \
             navigation.navigate('https://ex/c').finished.then(function(){ fin = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("corrio").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("fin").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn navigation_back_mueve_current() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var origen = navigation.currentEntry.url; navigation.navigate('https://ex/d'); navigation.back();")
            .expect("e");
        assert_eq!(rt.eval("navigation.currentEntry.url === origen").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigation.canGoForward").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.139 — View Transitions API ----

    #[test]
    fn view_transition_devuelve_objeto_con_promesas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var vt = document.startViewTransition(function(){});").expect("e");
        assert_eq!(rt.eval("typeof vt.ready.then").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof vt.finished.then").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof vt.updateCallbackDone.then").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof vt.skipTransition").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn view_transition_corre_callback_y_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var corrio = false, fin = false; \
             var vt = document.startViewTransition(function(){ corrio = true; }); \
             vt.finished.then(function(){ fin = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("corrio").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("fin").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn view_transition_skip_no_rompe_finished() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var fin = false; \
             var vt = document.startViewTransition(function(){}); \
             vt.skipTransition(); \
             vt.ready.catch(function(){}); \
             vt.finished.then(function(){ fin = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("fin").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn view_transition_callback_que_lanza_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; \
             var vt = document.startViewTransition(function(){ throw new Error('boom'); }); \
             vt.finished.catch(function(e){ err = e.message; }); \
             vt.updateCallbackDone.catch(function(){});",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("boom".into()));
    }

    // ---- Fase 7.140 — Cookie Store API ----

    #[test]
    fn cookie_store_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof cookieStore").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("typeof cookieStore.get").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof cookieStore.set").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn cookie_store_set_y_get_comparten_jar_con_document_cookie() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var v = null; \
             cookieStore.set('tema', 'oscuro'); \
             cookieStore.get('tema').then(function(c){ v = c ? c.value : null; });",
        )
        .expect("e");
        assert_eq!(rt.eval("v").expect("e"), JsValue::String("oscuro".into()));
        // El mismo jar que document.cookie (Fase 7.90).
        assert_eq!(
            rt.eval("__puriy_cookie_get().indexOf('tema=oscuro') >= 0").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn cookie_store_get_all_lista() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = -1; \
             cookieStore.set('a', '1'); cookieStore.set('b', '2'); \
             cookieStore.getAll().then(function(list){ n = list.length; });",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn cookie_store_delete_y_change_event() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var borrado = null, despues = 'x'; \
             cookieStore.set('s', 'v'); \
             cookieStore.addEventListener('change', function(e){ if (e.deleted.length) borrado = e.deleted[0].name; }); \
             cookieStore.delete('s'); \
             cookieStore.get('s').then(function(c){ despues = c; });",
        )
        .expect("e");
        assert_eq!(rt.eval("borrado").expect("e"), JsValue::String("s".into()));
        assert_eq!(rt.eval("despues").expect("e"), JsValue::Null);
    }

    #[test]
    fn cookie_store_change_event_en_set() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var cambiado = null; \
             cookieStore.onchange = function(e){ if (e.changed.length) cambiado = e.changed[0].value; }; \
             cookieStore.set('k', 'nuevo');",
        )
        .expect("e");
        assert_eq!(rt.eval("cambiado").expect("e"), JsValue::String("nuevo".into()));
    }

    // ---- Fase 7.141 — IndexedDB ----

    #[test]
    fn indexeddb_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof indexedDB").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("typeof indexedDB.open").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof IDBKeyRange.bound").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn indexeddb_open_dispara_upgradeneeded_y_success() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var fases = []; \
             var req = indexedDB.open('t_open', 1); \
             req.onupgradeneeded = function(){ fases.push('up'); req.result.createObjectStore('s', {keyPath:'id'}); }; \
             req.onsuccess = function(){ fases.push('ok'); };",
        )
        .expect("e");
        assert_eq!(rt.eval("fases.join(',')").expect("e"), JsValue::String("up,ok".into()));
    }

    #[test]
    fn indexeddb_add_y_get() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var nombre = null; \
             var req = indexedDB.open('t_add', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {keyPath:'id'}); }; \
             req.onsuccess = function(){ \
               var db = req.result; \
               db.transaction('s','readwrite').objectStore('s').add({id:5, nombre:'eva'}); \
               var g = db.transaction('s').objectStore('s').get(5); \
               g.onsuccess = function(){ nombre = g.result ? g.result.nombre : null; }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("nombre").expect("e"), JsValue::String("eva".into()));
    }

    #[test]
    fn indexeddb_put_sobrescribe() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var val = null; \
             var req = indexedDB.open('t_put', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {keyPath:'id'}); }; \
             req.onsuccess = function(){ \
               var db = req.result; var st = db.transaction('s','readwrite').objectStore('s'); \
               st.put({id:1, v:'a'}); st.put({id:1, v:'b'}); \
               var g = db.transaction('s').objectStore('s').get(1); \
               g.onsuccess = function(){ val = g.result.v; }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("val").expect("e"), JsValue::String("b".into()));
    }

    #[test]
    fn indexeddb_autoincrement() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var k1 = null, k2 = null; \
             var req = indexedDB.open('t_auto', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {autoIncrement:true}); }; \
             req.onsuccess = function(){ \
               var st = req.result.transaction('s','readwrite').objectStore('s'); \
               var a = st.add({x:1}); a.onsuccess = function(){ k1 = a.result; }; \
               var b = st.add({x:2}); b.onsuccess = function(){ k2 = b.result; }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("k1").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("k2").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn indexeddb_getall_ordenado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var orden = null; \
             var req = indexedDB.open('t_all', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {keyPath:'id'}); }; \
             req.onsuccess = function(){ \
               var db = req.result; var st = db.transaction('s','readwrite').objectStore('s'); \
               st.add({id:3}); st.add({id:1}); st.add({id:2}); \
               var g = db.transaction('s').objectStore('s').getAll(); \
               g.onsuccess = function(){ orden = g.result.map(function(r){ return r.id; }).join(','); }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("orden").expect("e"), JsValue::String("1,2,3".into()));
    }

    #[test]
    fn indexeddb_delete_y_count() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = -1; \
             var req = indexedDB.open('t_del', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {keyPath:'id'}); }; \
             req.onsuccess = function(){ \
               var db = req.result; var st = db.transaction('s','readwrite').objectStore('s'); \
               st.add({id:1}); st.add({id:2}); st.add({id:3}); st.delete(2); \
               var c = db.transaction('s').objectStore('s').count(); \
               c.onsuccess = function(){ n = c.result; }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn indexeddb_index_get() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var id = null; \
             var req = indexedDB.open('t_idx', 1); \
             req.onupgradeneeded = function(){ \
               var st = req.result.createObjectStore('p', {keyPath:'id'}); \
               st.createIndex('byName', 'name', {unique:false}); \
             }; \
             req.onsuccess = function(){ \
               var db = req.result; var st = db.transaction('p','readwrite').objectStore('p'); \
               st.add({id:1, name:'ana'}); st.add({id:2, name:'beto'}); \
               var g = db.transaction('p').objectStore('p').index('byName').get('beto'); \
               g.onsuccess = function(){ id = g.result ? g.result.id : null; }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("id").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn indexeddb_cursor_itera_en_orden() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var keys = []; \
             var req = indexedDB.open('t_cur', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {keyPath:'id'}); }; \
             req.onsuccess = function(){ \
               var db = req.result; var st = db.transaction('s','readwrite').objectStore('s'); \
               st.add({id:3}); st.add({id:1}); st.add({id:2}); \
               var c = db.transaction('s').objectStore('s').openCursor(); \
               c.onsuccess = function(){ var cur = c.result; if (cur) { keys.push(cur.key); cur.continue(); } }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("keys.join(',')").expect("e"), JsValue::String("1,2,3".into()));
    }

    #[test]
    fn indexeddb_keyrange_bound() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = -1; \
             var req = indexedDB.open('t_kr', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {keyPath:'id'}); }; \
             req.onsuccess = function(){ \
               var db = req.result; var st = db.transaction('s','readwrite').objectStore('s'); \
               for (var i = 1; i <= 5; i++) st.add({id:i}); \
               var g = db.transaction('s').objectStore('s').getAll(IDBKeyRange.bound(2, 4)); \
               g.onsuccess = function(){ n = g.result.length; }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(3.0));
    }

    #[test]
    fn indexeddb_cmp() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("indexedDB.cmp(1, 2)").expect("e"), JsValue::Number(-1.0));
        assert_eq!(rt.eval("indexedDB.cmp(2, 2)").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("indexedDB.cmp(3, 1)").expect("e"), JsValue::Number(1.0));
        // number < string en el orden de claves
        assert_eq!(rt.eval("indexedDB.cmp(9, 'a')").expect("e"), JsValue::Number(-1.0));
    }

    #[test]
    fn indexeddb_persiste_entre_conexiones() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var v = null; \
             var r1 = indexedDB.open('t_persist', 1); \
             r1.onupgradeneeded = function(){ r1.result.createObjectStore('s', {keyPath:'id'}); }; \
             r1.onsuccess = function(){ \
               var db = r1.result; \
               db.transaction('s','readwrite').objectStore('s').add({id:1, v:'guardado'}); \
               db.close(); \
               var r2 = indexedDB.open('t_persist'); \
               r2.onsuccess = function(){ \
                 var g = r2.result.transaction('s').objectStore('s').get(1); \
                 g.onsuccess = function(){ v = g.result ? g.result.v : null; }; \
               }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("v").expect("e"), JsValue::String("guardado".into()));
    }

    #[test]
    fn indexeddb_transaction_oncomplete() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var completo = false; \
             var req = indexedDB.open('t_tx', 1); \
             req.onupgradeneeded = function(){ req.result.createObjectStore('s', {autoIncrement:true}); }; \
             req.onsuccess = function(){ \
               var tx = req.result.transaction('s','readwrite'); \
               tx.objectStore('s').add({a:1}); \
               tx.oncomplete = function(){ completo = true; }; \
             };",
        )
        .expect("e");
        assert_eq!(rt.eval("completo").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.142 — WebRTC ----

    #[test]
    fn rtc_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof RTCPeerConnection").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof RTCSessionDescription").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof RTCIceCandidate").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("RTCPeerConnection === webkitRTCPeerConnection").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn rtc_create_offer_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var tipo = null, tieneSdp = false; \
             var pc = new RTCPeerConnection(); \
             pc.createOffer().then(function(o){ tipo = o.type; tieneSdp = o.sdp.length > 0; });",
        )
        .expect("e");
        assert_eq!(rt.eval("tipo").expect("e"), JsValue::String("offer".into()));
        assert_eq!(rt.eval("tieneSdp").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn rtc_set_local_description_cambia_signaling_state() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var estado = null; \
             var pc = new RTCPeerConnection(); \
             pc.createOffer().then(function(o){ \
                return pc.setLocalDescription(o); \
             }).then(function(){ estado = pc.signalingState; });",
        )
        .expect("e");
        assert_eq!(rt.eval("estado").expect("e"), JsValue::String("have-local-offer".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'rtc-local-description'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn rtc_data_channel_abre_y_envia() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var abierto = false; \
             var pc = new RTCPeerConnection(); \
             var ch = pc.createDataChannel('chat'); \
             ch.onopen = function(){ abierto = true; ch.send('hola'); };",
        )
        .expect("e");
        assert_eq!(rt.eval("abierto").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ch.readyState").expect("e"), JsValue::String("open".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'rtc-datachannel-send'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn rtc_ice_candidate_host_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var cand = null; \
             var pc = new RTCPeerConnection(); \
             pc.onicecandidate = function(ev){ cand = ev.candidate ? ev.candidate.candidate : null; };",
        )
        .expect("e");
        rt.eval("__puriy_rtc_ice_candidate(1, { candidate: 'candidate:1 1 udp 2 1.2.3.4 5 typ host' });")
            .expect("e");
        assert_eq!(
            rt.eval("cand").expect("e"),
            JsValue::String("candidate:1 1 udp 2 1.2.3.4 5 typ host".into())
        );
    }

    #[test]
    fn rtc_state_host_dispara_connectionstatechange() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var est = null; \
             var pc = new RTCPeerConnection(); \
             pc.onconnectionstatechange = function(){ est = pc.connectionState; };",
        )
        .expect("e");
        rt.eval("__puriy_rtc_state(1, 'connection', 'connected');").expect("e");
        assert_eq!(rt.eval("est").expect("e"), JsValue::String("connected".into()));
    }

    #[test]
    fn rtc_incoming_datachannel() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var label = null, estado = null; \
             var pc = new RTCPeerConnection(); \
             pc.ondatachannel = function(ev){ label = ev.channel.label; estado = ev.channel.readyState; };",
        )
        .expect("e");
        rt.eval("__puriy_rtc_datachannel(1, { label: 'entrante' });").expect("e");
        assert_eq!(rt.eval("label").expect("e"), JsValue::String("entrante".into()));
        assert_eq!(rt.eval("estado").expect("e"), JsValue::String("open".into()));
    }

    #[test]
    fn rtc_datachannel_message_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var msg = null; \
             var pc = new RTCPeerConnection(); \
             var ch = pc.createDataChannel('d'); \
             ch.onmessage = function(ev){ msg = ev.data; };",
        )
        .expect("e");
        rt.eval("__puriy_rtc_datachannel_message(1, 'd', 'pong');").expect("e");
        assert_eq!(rt.eval("msg").expect("e"), JsValue::String("pong".into()));
    }

    #[test]
    fn rtc_close() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var pc = new RTCPeerConnection(); pc.close();",
        )
        .expect("e");
        assert_eq!(rt.eval("pc.signalingState").expect("e"), JsValue::String("closed".into()));
        assert_eq!(rt.eval("pc.connectionState").expect("e"), JsValue::String("closed".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'rtc-close'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn rtc_session_description_tojson() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var d = new RTCSessionDescription({ type: 'answer', sdp: 'v=0' }); var j = d.toJSON();")
            .expect("e");
        assert_eq!(rt.eval("j.type").expect("e"), JsValue::String("answer".into()));
        assert_eq!(rt.eval("j.sdp").expect("e"), JsValue::String("v=0".into()));
    }

    #[test]
    fn rtc_ice_candidate_tojson() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var c = new RTCIceCandidate({ candidate: 'abc', sdpMid: '0', sdpMLineIndex: 0 }); var j = c.toJSON();")
            .expect("e");
        assert_eq!(rt.eval("j.candidate").expect("e"), JsValue::String("abc".into()));
        assert_eq!(rt.eval("j.sdpMid").expect("e"), JsValue::String("0".into()));
        assert_eq!(rt.eval("j.sdpMLineIndex").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn rtc_add_ice_candidate_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ok = false; \
             var pc = new RTCPeerConnection(); \
             pc.addIceCandidate({ candidate: 'x' }).then(function(){ ok = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'rtc-add-ice'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    // ---- Fase 7.143 — Web Workers ----

    #[test]
    fn workers_existen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof Worker").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof SharedWorker").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn worker_spawn_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var w = new Worker('/wkr.js', { name: 'calc' });").expect("e");
        assert_eq!(rt.eval("w.url").expect("e"), JsValue::String("/wkr.js".into()));
        assert_eq!(rt.eval("w.name").expect("e"), JsValue::String("calc".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'worker-spawn'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn worker_post_message_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var w = new Worker('/w.js'); w.postMessage({ op: 'sum', a: 2, b: 3 });").expect("e");
        assert_eq!(
            rt.eval(
                "__puriy_dirty.some(function(d){ return d.kind === 'worker-message' && d.value.indexOf('sum') >= 0; })"
            )
            .expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn worker_message_host_dispara_onmessage() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var recibido = null; \
             var w = new Worker('/w.js'); \
             w.onmessage = function(ev){ recibido = ev.data; };",
        )
        .expect("e");
        rt.eval("__puriy_worker_message(1, 42);").expect("e");
        assert_eq!(rt.eval("recibido").expect("e"), JsValue::Number(42.0));
    }

    #[test]
    fn worker_message_via_addeventlistener() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = null; \
             var w = new Worker('/w.js'); \
             w.addEventListener('message', function(ev){ r = ev.data; });",
        )
        .expect("e");
        rt.eval("__puriy_worker_message(1, 'hola');").expect("e");
        assert_eq!(rt.eval("r").expect("e"), JsValue::String("hola".into()));
    }

    #[test]
    fn worker_error_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var msg = null; \
             var w = new Worker('/w.js'); \
             w.onerror = function(ev){ msg = ev.message; };",
        )
        .expect("e");
        rt.eval("__puriy_worker_error(1, 'boom');").expect("e");
        assert_eq!(rt.eval("msg").expect("e"), JsValue::String("boom".into()));
    }

    #[test]
    fn worker_terminate_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var w = new Worker('/w.js'); w.terminate();").expect("e");
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'worker-terminate'; })").expect("e"),
            JsValue::Bool(true)
        );
        // tras terminate, el host ya no entrega
        assert_eq!(rt.eval("__puriy_worker_message(1, 1)").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn shared_worker_tiene_port() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var sw = new SharedWorker('/sw.js');").expect("e");
        assert_eq!(rt.eval("sw.port instanceof MessagePort").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'sharedworker-spawn'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn shared_worker_port_post_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var sw = new SharedWorker('/sw.js'); sw.port.postMessage('ping');").expect("e");
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'sharedworker-message'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn shared_worker_port_recibe_del_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var r = null; \
             var sw = new SharedWorker('/sw.js'); \
             sw.port.onmessage = function(ev){ r = ev.data; };",
        )
        .expect("e");
        // el SharedWorker es el segundo worker creado en este runtime fresco → id 1
        rt.eval("__puriy_sharedworker_message(1, 'desde-sw');").expect("e");
        assert_eq!(rt.eval("r").expect("e"), JsValue::String("desde-sw".into()));
    }

