//! Tests de Fullscreen, PointerLock, WebBluetooth, FileSystemAccess, WebAnimations, WebAuthn, WebTransport, PushAPI, BackgroundSync, Sensors, NFC.
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
