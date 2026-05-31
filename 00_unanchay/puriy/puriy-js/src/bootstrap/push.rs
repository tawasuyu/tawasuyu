pub(crate) const PUSH_BOOTSTRAP: &str = r#"
// Fase 7.130 — Push API. Cuelga de ServiceWorkerRegistration (Fase 7.100):
// `registration.pushManager` es un PushManager (getter perezoso memoizado por
// registro). El servicio push real es del chrome (wiring nativo PENDIENTE):
// `subscribe(options)` publica kind: 'push-subscribe' (value `<id>`) y el chrome
// resuelve con __puriy_push_resolve(id, info) → PushSubscription o rechaza con
// __puriy_push_reject(id, name, msg) → NotAllowedError — patrón pending-id como
// share/serviceworker. `getSubscription()` devuelve la última suscripción o null;
// `permissionState()` es host-decided (default 'prompt', el chrome lo fija con
// __puriy_set_push_permission(state)).
(function() {
    if (globalThis.PushManager != null) return;
    if (globalThis.ServiceWorkerRegistration == null) return;  // requiere Fase 7.100
    globalThis.__puriy_push_pending = globalThis.__puriy_push_pending || {};
    globalThis.__puriy_push_next_id = globalThis.__puriy_push_next_id || 1;

    function PushSubscription(info, mgr) {
        info = info || {};
        this.endpoint = String(info.endpoint != null ? info.endpoint : '');
        this.expirationTime = (info.expirationTime != null) ? info.expirationTime : null;
        this.options = info.options || { userVisibleOnly: true, applicationServerKey: null };
        this._keys = info.keys || {};
        this._mgr = mgr;
    }
    PushSubscription.prototype.getKey = function(name) {
        var v = this._keys[name];
        return (v != null) ? v : null;
    };
    PushSubscription.prototype.toJSON = function() {
        return { endpoint: this.endpoint, expirationTime: this.expirationTime, keys: this._keys };
    };
    PushSubscription.prototype.unsubscribe = function() {
        if (this._mgr && this._mgr.__current === this) this._mgr.__current = null;
        return Promise.resolve(true);
    };
    globalThis.PushSubscription = PushSubscription;

    function PushManager() { this.__current = null; }
    PushManager.supportedContentEncodings = ['aes128gcm'];
    PushManager.prototype.subscribe = function(options) {
        var self = this;
        var id = globalThis.__puriy_push_next_id++;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'push-subscribe', value: String(id) });
        return new Promise(function(res, rej) {
            globalThis.__puriy_push_pending[id] = { resolve: res, reject: rej, mgr: self, options: options || {} };
        });
    };
    PushManager.prototype.getSubscription = function() {
        return Promise.resolve(this.__current);
    };
    PushManager.prototype.permissionState = function(options) {
        return Promise.resolve(globalThis.__puriy_push_permission || 'prompt');
    };
    globalThis.PushManager = PushManager;

    Object.defineProperty(globalThis.ServiceWorkerRegistration.prototype, 'pushManager', {
        configurable: true,
        get: function() {
            if (this.__pushManager == null) this.__pushManager = new PushManager();
            return this.__pushManager;
        }
    });

    globalThis.__puriy_push_resolve = function(id, info) {
        var p = globalThis.__puriy_push_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_push_pending[id];
        info = info || {};
        if (info.options == null) info.options = p.options;
        var sub = new PushSubscription(info, p.mgr);
        p.mgr.__current = sub;
        p.resolve(sub);
        return true;
    };
    globalThis.__puriy_push_reject = function(id, name, message) {
        var p = globalThis.__puriy_push_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_push_pending[id];
        p.reject(new globalThis.DOMException(
            (message != null) ? String(message) : 'Push subscribe falló',
            (name != null) ? String(name) : 'NotAllowedError'));
        return true;
    };
    globalThis.__puriy_set_push_permission = function(state) {
        globalThis.__puriy_push_permission = String(state);
        return true;
    };
    void 0;
})();
"#;
