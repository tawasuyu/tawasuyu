pub(crate) const EME_BOOTSTRAP: &str = r#"
// Fase 7.148 — Encrypted Media Extensions (`navigator.requestMediaKeySystemAccess`
// + `MediaKeySystemAccess`/`MediaKeys`/`MediaKeySession`/`MediaKeyStatusMap`).
// DRM para medios protegidos (Widevine/PlayReady/FairPlay/Clear Key). El handshake
// de licencia es del chrome (wiring PENDIENTE): la negociación de capacidades y la
// máquina de sesión son JS-puras, pero las licencias reales las inyecta el host.
//   · `requestMediaKeySystemAccess(keySystem, configs)` resuelve con un
//     `MediaKeySystemAccess` (rechaza `NotSupportedError` para sistemas no soportados;
//     el chrome amplía la lista soportada con `__puriy_eme_set_supported(list)`).
//   · `session.generateRequest(initDataType, initData)` publica kind: 'eme-message'
//     y el chrome responde con `__puriy_eme_message(sessionId, msgType, message)` →
//     evento `message` (el app manda eso al servidor de licencias).
//   · `session.update(response)` publica 'eme-update'; el chrome confirma las claves
//     con `__puriy_eme_keystatus(sessionId, keyId, status)` → `keystatuseschange`.
(function() {
    if (globalThis.MediaKeys != null) return;
    var GS = String.fromCharCode(0x1D);
    var nav = globalThis.navigator = globalThis.navigator || {};
    var nextId = 1;
    var sessions = {};
    // Sistemas DRM "soportados" por defecto; el chrome amplía con __puriy_eme_set_supported.
    var supported = { 'org.w3.clearkey': true };
    globalThis.__puriy_eme_set_supported = function(list) {
        if (Array.isArray(list)) for (var i = 0; i < list.length; i++) supported[String(list[i])] = true;
    };

    function emit(self, type, ev) {
        ev = ev || { type: type };
        var h = self['on' + type];
        if (typeof h === 'function') { try { h.call(self, ev); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; } }
        self.dispatchEvent(ev);
    }

    // ---- MediaKeyStatusMap (Map-like de keyId → status) ----
    function MediaKeyStatusMap() { this._m = []; this.size = 0; }
    MediaKeyStatusMap.prototype._set = function(keyId, status) {
        for (var i = 0; i < this._m.length; i++) if (this._m[i][0] === keyId) { this._m[i][1] = status; return; }
        this._m.push([keyId, status]); this.size = this._m.length;
    };
    MediaKeyStatusMap.prototype.has = function(keyId) {
        for (var i = 0; i < this._m.length; i++) if (this._m[i][0] === keyId) return true;
        return false;
    };
    MediaKeyStatusMap.prototype.get = function(keyId) {
        for (var i = 0; i < this._m.length; i++) if (this._m[i][0] === keyId) return this._m[i][1];
        return undefined;
    };
    MediaKeyStatusMap.prototype.forEach = function(cb, thisArg) {
        for (var i = 0; i < this._m.length; i++) cb.call(thisArg, this._m[i][1], this._m[i][0], this);
    };

    // ---- MediaKeySession ----
    function MediaKeySession(sessionType) {
        globalThis.EventTarget.call(this);
        this._id = nextId++;
        sessions[this._id] = this;
        this.sessionId = '';
        this._sessionType = sessionType || 'temporary';
        this.expiration = NaN;
        this.keyStatuses = new MediaKeyStatusMap();
        this._closed = false;
        var self = this;
        this.closed = new Promise(function(res) { self._resolveClosed = res; });
        this.onmessage = null; this.onkeystatuseschange = null;
    }
    MediaKeySession.prototype = Object.create(globalThis.EventTarget.prototype);
    MediaKeySession.prototype.constructor = MediaKeySession;
    MediaKeySession.prototype.generateRequest = function(initDataType, initData) {
        if (this._closed) return Promise.reject(new globalThis.DOMException('sesión cerrada', 'InvalidStateError'));
        this.sessionId = this.sessionId || ('puriy-eme-' + this._id);
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'eme-message',
            value: this._id + GS + 'generaterequest' + GS + String(initDataType != null ? initDataType : '') });
        return Promise.resolve();
    };
    MediaKeySession.prototype.load = function(sessionId) {
        this.sessionId = String(sessionId);
        return Promise.resolve(true);
    };
    MediaKeySession.prototype.update = function(response) {
        if (this._closed) return Promise.reject(new globalThis.DOMException('sesión cerrada', 'InvalidStateError'));
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'eme-update', value: String(this._id) });
        return Promise.resolve();
    };
    MediaKeySession.prototype.remove = function() {
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'eme-remove', value: String(this._id) });
        return Promise.resolve();
    };
    MediaKeySession.prototype.close = function() {
        if (!this._closed) { this._closed = true; if (this._resolveClosed) this._resolveClosed(); delete sessions[this._id]; }
        return Promise.resolve();
    };

    // ---- MediaKeys ----
    function MediaKeys(keySystem) { this._keySystem = keySystem; this._serverCert = null; }
    MediaKeys.prototype.createSession = function(sessionType) {
        return new MediaKeySession(sessionType || 'temporary');
    };
    MediaKeys.prototype.setServerCertificate = function(cert) {
        this._serverCert = cert;
        return Promise.resolve(true);
    };

    // ---- MediaKeySystemAccess ----
    function MediaKeySystemAccess(keySystem, config) {
        this.keySystem = keySystem;
        this._config = config || {};
    }
    MediaKeySystemAccess.prototype.getConfiguration = function() { return this._config; };
    MediaKeySystemAccess.prototype.createMediaKeys = function() {
        return Promise.resolve(new MediaKeys(this.keySystem));
    };

    // ---- navigator.requestMediaKeySystemAccess ----
    nav.requestMediaKeySystemAccess = function(keySystem, configs) {
        var ks = String(keySystem);
        if (!supported[ks]) {
            return Promise.reject(new globalThis.DOMException('sistema de claves no soportado: ' + ks, 'NotSupportedError'));
        }
        if (!Array.isArray(configs) || configs.length === 0) {
            return Promise.reject(new globalThis.DOMException('configuración requerida', 'TypeError'));
        }
        // Acepta la primera configuración (negociación mínima).
        var cfg = configs[0] || {};
        var resolved = {
            label: cfg.label || '',
            initDataTypes: cfg.initDataTypes || ['cenc'],
            audioCapabilities: cfg.audioCapabilities || [],
            videoCapabilities: cfg.videoCapabilities || [],
            distinctiveIdentifier: cfg.distinctiveIdentifier || 'optional',
            persistentState: cfg.persistentState || 'optional',
            sessionTypes: cfg.sessionTypes || ['temporary']
        };
        return Promise.resolve(new MediaKeySystemAccess(ks, resolved));
    };

    // ---- Hooks del host ----
    globalThis.__puriy_eme_message = function(sessionId, msgType, message) {
        var s = sessions[sessionId]; if (!s) return false;
        var bytes = (message instanceof Uint8Array) ? message : new globalThis.TextEncoder().encode(String(message || ''));
        emit(s, 'message', { type: 'message', messageType: msgType != null ? msgType : 'license-request', message: bytes.buffer });
        return true;
    };
    globalThis.__puriy_eme_keystatus = function(sessionId, keyId, status) {
        var s = sessions[sessionId]; if (!s) return false;
        s.keyStatuses._set(String(keyId), String(status != null ? status : 'usable'));
        emit(s, 'keystatuseschange', { type: 'keystatuseschange' });
        return true;
    };

    globalThis.MediaKeys = MediaKeys;
    globalThis.MediaKeySession = MediaKeySession;
    globalThis.MediaKeySystemAccess = MediaKeySystemAccess;
    globalThis.MediaKeyStatusMap = MediaKeyStatusMap;
    void 0;
})();
"#;
