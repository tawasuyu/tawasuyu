pub(crate) const BACKGROUNDFETCH_BOOTSTRAP: &str = r#"
// Fase 7.159 — Background Fetch API. Cuelga de ServiceWorkerRegistration (Fase
// 7.100): `registration.backgroundFetch` es un `BackgroundFetchManager` (getter
// perezoso memoizado por registro, mismo molde que pushManager 7.130 y
// sync/periodicSync 7.131). Permite a una PWA bajar conjuntos grandes de recursos
// (vídeos, niveles de juego) que sobreviven al cierre de la pestaña. La descarga
// real es del chrome (wiring PENDIENTE): `fetch(id, requests, options)` publica
// kind: 'backgroundfetch' (value `<uid> GS <id>`) y resuelve ya con una
// `BackgroundFetchRegistration`; el chrome reporta avance vía
// __puriy_backgroundfetch_progress(uid, info) (dispara `progress`) y cierre con
// result 'success'/'failure'.
(function() {
    if (globalThis.BackgroundFetchManager != null) return;
    if (globalThis.ServiceWorkerRegistration == null) return;  // requiere Fase 7.100
    var GS = String.fromCharCode(0x1D);
    globalThis.__puriy_bgf_registry = globalThis.__puriy_bgf_registry || {};
    globalThis.__puriy_bgf_next_uid = globalThis.__puriy_bgf_next_uid || 1;

    function BackgroundFetchRecord(request) {
        this.request = request;
        this.responseReady = Promise.resolve(null);  // el chrome la reemplaza por la Response real
    }
    globalThis.BackgroundFetchRecord = BackgroundFetchRecord;

    function BackgroundFetchRegistration(id, requests, options) {
        var et = new globalThis.EventTarget();
        for (var m in et) { if (typeof et[m] === 'function') this[m] = et[m].bind(et); }
        options = options || {};
        this.id = String(id);
        this._uid = globalThis.__puriy_bgf_next_uid++;
        this._requests = requests;
        this.uploadTotal = 0;
        this.uploaded = 0;
        this.downloadTotal = (options.downloadTotal != null) ? (options.downloadTotal | 0) : 0;
        this.downloaded = 0;
        this.result = '';            // ''|'success'|'failure'
        this.failureReason = '';     // ''|'aborted'|'bad-status'|'fetch-error'|...
        this.recordsAvailable = true;
        this.onprogress = null;
        globalThis.__puriy_bgf_registry[this._uid] = this;
    }
    BackgroundFetchRegistration.prototype.abort = function() {
        this.result = 'failure';
        this.failureReason = 'aborted';
        this.recordsAvailable = false;
        if (this._mgr) delete this._mgr._registrations[this.id];
        delete globalThis.__puriy_bgf_registry[this._uid];
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'backgroundfetch-abort', value: String(this._uid) });
        return Promise.resolve(true);
    };
    BackgroundFetchRegistration.prototype.match = function(request) {
        var recs = this._requests || [];
        return Promise.resolve(recs.length ? new BackgroundFetchRecord(recs[0]) : undefined);
    };
    BackgroundFetchRegistration.prototype.matchAll = function(request) {
        var recs = this._requests || [];
        return Promise.resolve(recs.map(function(r) { return new BackgroundFetchRecord(r); }));
    };
    globalThis.BackgroundFetchRegistration = BackgroundFetchRegistration;

    function BackgroundFetchManager() { this._registrations = {}; }
    BackgroundFetchManager.prototype.fetch = function(id, requests, options) {
        if (id == null || String(id) === '') {
            return Promise.reject(new TypeError('backgroundFetch.fetch requiere un id'));
        }
        id = String(id);
        if (requests == null) {
            return Promise.reject(new TypeError('backgroundFetch.fetch requiere requests'));
        }
        var list = Array.isArray(requests) ? requests.slice() : [requests];
        if (list.length === 0) {
            return Promise.reject(new TypeError('backgroundFetch.fetch requiere al menos un request'));
        }
        if (this._registrations[id]) {
            return Promise.reject(new TypeError("ya existe un background fetch con id '" + id + "'"));
        }
        var reg = new BackgroundFetchRegistration(id, list, options);
        reg._mgr = this;
        this._registrations[id] = reg;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'backgroundfetch', value: reg._uid + GS + id });
        return Promise.resolve(reg);
    };
    BackgroundFetchManager.prototype.get = function(id) {
        return Promise.resolve(this._registrations[String(id)] || undefined);
    };
    BackgroundFetchManager.prototype.getIds = function() {
        return Promise.resolve(Object.keys(this._registrations));
    };
    globalThis.BackgroundFetchManager = BackgroundFetchManager;

    Object.defineProperty(globalThis.ServiceWorkerRegistration.prototype, 'backgroundFetch', {
        configurable: true,
        get: function() {
            if (this.__backgroundFetch == null) this.__backgroundFetch = new BackgroundFetchManager();
            return this.__backgroundFetch;
        }
    });

    // El chrome reporta avance/cierre de la descarga en background.
    globalThis.__puriy_backgroundfetch_progress = function(uid, info) {
        var reg = globalThis.__puriy_bgf_registry[uid];
        if (!reg) return false;
        info = info || {};
        if (info.downloaded != null) reg.downloaded = info.downloaded | 0;
        if (info.downloadTotal != null) reg.downloadTotal = info.downloadTotal | 0;
        if (info.uploaded != null) reg.uploaded = info.uploaded | 0;
        if (info.result != null) reg.result = String(info.result);
        if (info.failureReason != null) reg.failureReason = String(info.failureReason);
        var ev = (typeof globalThis.Event === 'function') ? new globalThis.Event('progress') : { type: 'progress' };
        ev.target = reg;
        if (typeof reg.onprogress === 'function') { try { reg.onprogress(ev); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; } }
        if (typeof reg.dispatchEvent === 'function') reg.dispatchEvent(ev);
        return true;
    };
    void 0;
})();
"#;
