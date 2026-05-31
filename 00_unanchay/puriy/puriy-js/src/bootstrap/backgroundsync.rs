pub(crate) const BACKGROUNDSYNC_BOOTSTRAP: &str = r#"
// Fase 7.131 — Background Sync + Periodic Background Sync. Cuelgan de
// ServiceWorkerRegistration (Fase 7.100): `registration.sync` es un SyncManager y
// `registration.periodicSync` un PeriodicSyncManager (getters perezosos memoizados
// por registro). El disparo real del evento `sync`/`periodicsync` en el worker es
// del chrome (wiring nativo PENDIENTE): `register(tag)` publica
// kind: 'sync-register' (value `<tag>`) / 'periodicsync-register'
// (value `<tag> GS <minInterval>`) y resuelve; los tags quedan en una lista local
// que `getTags()` devuelve. `unregister(tag)` (sólo periodicSync) los quita y
// publica 'periodicsync-unregister'.
(function() {
    if (globalThis.SyncManager != null) return;
    if (globalThis.ServiceWorkerRegistration == null) return;  // requiere Fase 7.100
    var GS = String.fromCharCode(0x1D);

    function SyncManager() { this._tags = []; }
    SyncManager.prototype.register = function(tag) {
        tag = String(tag);
        if (this._tags.indexOf(tag) === -1) this._tags.push(tag);
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'sync-register', value: tag });
        return Promise.resolve();
    };
    SyncManager.prototype.getTags = function() { return Promise.resolve(this._tags.slice()); };
    globalThis.SyncManager = SyncManager;

    function PeriodicSyncManager() { this._tags = []; }
    PeriodicSyncManager.prototype.register = function(tag, options) {
        tag = String(tag);
        if (this._tags.indexOf(tag) === -1) this._tags.push(tag);
        var minInterval = (options != null && options.minInterval != null) ? (options.minInterval | 0) : 0;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'periodicsync-register',
            value: tag + GS + String(minInterval)
        });
        return Promise.resolve();
    };
    PeriodicSyncManager.prototype.getTags = function() { return Promise.resolve(this._tags.slice()); };
    PeriodicSyncManager.prototype.unregister = function(tag) {
        tag = String(tag);
        var i = this._tags.indexOf(tag);
        if (i !== -1) this._tags.splice(i, 1);
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'periodicsync-unregister', value: tag });
        return Promise.resolve();
    };
    globalThis.PeriodicSyncManager = PeriodicSyncManager;

    Object.defineProperty(globalThis.ServiceWorkerRegistration.prototype, 'sync', {
        configurable: true,
        get: function() {
            if (this.__syncManager == null) this.__syncManager = new SyncManager();
            return this.__syncManager;
        }
    });
    Object.defineProperty(globalThis.ServiceWorkerRegistration.prototype, 'periodicSync', {
        configurable: true,
        get: function() {
            if (this.__periodicSyncManager == null) this.__periodicSyncManager = new PeriodicSyncManager();
            return this.__periodicSyncManager;
        }
    });
    void 0;
})();
"#;
