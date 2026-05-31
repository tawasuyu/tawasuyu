pub(crate) const SERVICEWORKER_BOOTSTRAP: &str = r#"
// Fase 7.100 — `navigator.serviceWorker` (stub fiel-pero-inerte). Las PWAs y
// libs de offline chequean `'serviceWorker' in navigator` y llaman a
// `register()` al cargar. Puriy no corre un worker en segundo plano todavía:
// `register(url)` publica una mutación `kind: 'serviceworker-register'` al
// chrome y devuelve una Promise que el chrome resuelve con
// `__puriy_serviceworker_resolve(id, scope)` (entregando un
// ServiceWorkerRegistration) o rechaza con `__puriy_serviceworker_reject(id,
// name, msg)` — mismo patrón pending-id que share/geolocation (wiring nativo
// pendiente). `serviceWorker` y la `registration` son EventTarget (Fase 7.76).
// `controller` es null (ningún SW controla la página); `getRegistration(s)`
// resuelven vacío hasta que el chrome registre algo.
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.serviceWorker != null) return;

    globalThis.__puriy_sw_pending = globalThis.__puriy_sw_pending || {};
    globalThis.__puriy_sw_next_id = globalThis.__puriy_sw_next_id || 1;

    function ServiceWorkerRegistration(scope) {
        globalThis.EventTarget.call(this);
        this.scope = scope;
        this.installing = null;
        this.waiting = null;
        this.active = null;
        this.updateViaCache = 'imports';
        this.onupdatefound = null;
    }
    ServiceWorkerRegistration.prototype = Object.create(globalThis.EventTarget.prototype);
    ServiceWorkerRegistration.prototype.constructor = ServiceWorkerRegistration;
    ServiceWorkerRegistration.prototype.update = function() { return Promise.resolve(this); };
    ServiceWorkerRegistration.prototype.unregister = function() { return Promise.resolve(true); };
    globalThis.ServiceWorkerRegistration = ServiceWorkerRegistration;

    function ServiceWorkerContainer() {
        globalThis.EventTarget.call(this);
        this.controller = null;
        this.oncontrollerchange = null;
        this.onmessage = null;
        var self = this;
        this.ready = new Promise(function(resolve) { self.__resolveReady = resolve; });
    }
    ServiceWorkerContainer.prototype = Object.create(globalThis.EventTarget.prototype);
    ServiceWorkerContainer.prototype.constructor = ServiceWorkerContainer;
    ServiceWorkerContainer.prototype.register = function(scriptURL, options) {
        var url = String(scriptURL);
        var scope = (options != null && options.scope != null)
            ? String(options.scope)
            : url.replace(/[^/]*$/, '');
        var id = globalThis.__puriy_sw_next_id++;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'serviceworker-register',
            value: id + '' + JSON.stringify({ url: url, scope: scope })
        });
        var self = this;
        return new Promise(function(resolve, reject) {
            globalThis.__puriy_sw_pending[id] = { resolve: resolve, reject: reject, container: self };
        });
    };
    ServiceWorkerContainer.prototype.getRegistration = function(clientURL) {
        return Promise.resolve(undefined);
    };
    ServiceWorkerContainer.prototype.getRegistrations = function() {
        return Promise.resolve([]);
    };
    ServiceWorkerContainer.prototype.startMessages = function() { /* no-op */ };
    globalThis.ServiceWorkerContainer = ServiceWorkerContainer;

    var container = new ServiceWorkerContainer();
    nav.serviceWorker = container;

    globalThis.__puriy_serviceworker_resolve = function(id, scope) {
        var p = globalThis.__puriy_sw_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_sw_pending[id];
        var reg = new ServiceWorkerRegistration((scope != null) ? String(scope) : '/');
        p.resolve(reg);
        if (p.container && typeof p.container.__resolveReady === 'function') {
            p.container.__resolveReady(reg);
        }
        return true;
    };
    globalThis.__puriy_serviceworker_reject = function(id, name, message) {
        var p = globalThis.__puriy_sw_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_sw_pending[id];
        p.reject(new globalThis.DOMException(
            (message != null) ? String(message) : 'ServiceWorker registration failed',
            (name != null) ? String(name) : 'SecurityError'));
        return true;
    };
})();
"#;
