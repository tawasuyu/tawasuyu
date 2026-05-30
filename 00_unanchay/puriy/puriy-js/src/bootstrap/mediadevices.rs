pub(crate) const MEDIADEVICES_BOOTSTRAP: &str = r#"
// Fase 7.101 — `navigator.mediaDevices` (cámara/micrófono, gated-by-permission).
// Las apps de videollamada/foto chequean `navigator.mediaDevices` y llaman
// `getUserMedia({video, audio})`. Puriy no tiene captura A/V: por defecto
// `getUserMedia`/`getDisplayMedia` rechazan con `NotAllowedError` (igual que un
// browser con permiso denegado), pero el chrome puede flippear la decisión con
// `__puriy_set_media_devices_permission(bool)` y poblar el listado con
// `__puriy_set_media_devices([...])` — mismo patrón host-driven que Permissions
// 7.93. `enumerateDevices` devuelve el listado (vacío por defecto). El stub es
// fiel para feature-detection y degrada con gracia; cuando se permita, sigue
// sin haber MediaStream real (wiring de captura pendiente), así que resuelve un
// stream vacío inerte. `mediaDevices` es un EventTarget (Fase 7.76, evento
// `devicechange`).
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.mediaDevices != null) return;

    var state = globalThis.__puriy_media_devices_state = globalThis.__puriy_media_devices_state || {
        permitido: false, dispositivos: []
    };

    function MediaStream(tracks) {
        globalThis.EventTarget.call(this);
        this.id = 'puriy-stream-' + (globalThis.__puriy_media_stream_id =
            (globalThis.__puriy_media_stream_id || 0) + 1);
        this.active = true;
        this.__tracks = tracks || [];
    }
    MediaStream.prototype = Object.create(globalThis.EventTarget.prototype);
    MediaStream.prototype.constructor = MediaStream;
    MediaStream.prototype.getTracks = function() { return this.__tracks.slice(); };
    MediaStream.prototype.getAudioTracks = function() {
        return this.__tracks.filter(function(t) { return t.kind === 'audio'; });
    };
    MediaStream.prototype.getVideoTracks = function() {
        return this.__tracks.filter(function(t) { return t.kind === 'video'; });
    };
    MediaStream.prototype.addTrack = function(t) { this.__tracks.push(t); };
    MediaStream.prototype.removeTrack = function(t) {
        var i = this.__tracks.indexOf(t);
        if (i >= 0) this.__tracks.splice(i, 1);
    };
    globalThis.MediaStream = MediaStream;

    function MediaDevices() {
        globalThis.EventTarget.call(this);
        this.ondevicechange = null;
    }
    MediaDevices.prototype = Object.create(globalThis.EventTarget.prototype);
    MediaDevices.prototype.constructor = MediaDevices;

    function capturar(constraints) {
        if (!globalThis.__puriy_media_devices_state.permitido) {
            return Promise.reject(new globalThis.DOMException(
                'Permission denied', 'NotAllowedError'));
        }
        // Permiso concedido pero sin captura real: stream inerte sin tracks.
        return Promise.resolve(new MediaStream([]));
    }
    MediaDevices.prototype.getUserMedia = function(constraints) {
        if (constraints == null || (constraints.video == null && constraints.audio == null)) {
            return Promise.reject(new TypeError(
                'getUserMedia requiere al menos video o audio'));
        }
        return capturar(constraints);
    };
    MediaDevices.prototype.getDisplayMedia = function(constraints) {
        return capturar(constraints);
    };
    MediaDevices.prototype.enumerateDevices = function() {
        return Promise.resolve(globalThis.__puriy_media_devices_state.dispositivos.slice());
    };
    MediaDevices.prototype.getSupportedConstraints = function() {
        return { width: true, height: true, aspectRatio: true, frameRate: true,
                 facingMode: true, deviceId: true, groupId: true };
    };
    globalThis.MediaDevices = MediaDevices;

    var devices = new MediaDevices();
    nav.mediaDevices = devices;

    globalThis.__puriy_set_media_devices_permission = function(allowed) {
        state.permitido = !!allowed;
        return true;
    };
    globalThis.__puriy_set_media_devices = function(list) {
        state.dispositivos = Array.isArray(list) ? list.slice() : [];
        var ev = new globalThis.Event('devicechange');
        if (typeof devices.ondevicechange === 'function') {
            try { devices.ondevicechange.call(devices, ev); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
        devices.dispatchEvent(ev);
        return true;
    };
})();
"#;
