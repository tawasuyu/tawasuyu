pub(crate) const CACHE_STORAGE_BOOTSTRAP: &str = r#"
// Fase 7.91 — Cache API (`caches` / CacheStorage + Cache). El almacén que usan
// los Service Workers para servir offline: `caches.open(name)` devuelve un
// `Cache` donde guardás pares Request→Response. Acá es in-memory por runtime
// (clave = URL del request); las Response se guardan/devuelven clonadas (Fase
// 7.55) para que cada lado tenga su propio body consumible una vez.
//
// Cargado DESPUÉS de `response`/`request`/`fetch` (los usa). `add`/`addAll`
// salen a la red vía fetch() — su promise sólo resuelve cuando el chrome
// completa el fetch (wiring de red pendiente), igual que el resto del front.
(function() {
    function reqKey(request) {
        if (typeof request === 'string') return request;
        if (request instanceof globalThis.Request) return request.url;
        if (request && request.url != null) return String(request.url);
        return String(request);
    }

    function Cache() { this._store = {}; this._order = []; }
    Cache.prototype.put = function(request, response) {
        var key = reqKey(request);
        var copy;
        try { copy = response.clone(); }
        catch (e) { return Promise.reject(new TypeError('Cache.put: el body de la Response ya fue usado')); }
        if (!Object.prototype.hasOwnProperty.call(this._store, key)) this._order.push(key);
        this._store[key] = copy;
        return Promise.resolve(undefined);
    };
    Cache.prototype.match = function(request) {
        var r = this._store[reqKey(request)];
        return Promise.resolve(r ? r.clone() : undefined);
    };
    Cache.prototype.matchAll = function(request) {
        var out = [];
        if (request == null) {
            for (var i = 0; i < this._order.length; i++) out.push(this._store[this._order[i]].clone());
        } else {
            var r = this._store[reqKey(request)];
            if (r) out.push(r.clone());
        }
        return Promise.resolve(out);
    };
    Cache.prototype.add = function(request) {
        var self = this;
        return globalThis.fetch(request).then(function(resp) {
            if (!resp.ok) throw new TypeError('Cache.add: la respuesta no fue ok (' + resp.status + ')');
            return self.put(request, resp);
        });
    };
    Cache.prototype.addAll = function(requests) {
        var self = this;
        var list = requests || [];
        var jobs = [];
        for (var i = 0; i < list.length; i++) jobs.push(self.add(list[i]));
        return Promise.all(jobs).then(function() { return undefined; });
    };
    Cache.prototype['delete'] = function(request) {
        var key = reqKey(request);
        if (Object.prototype.hasOwnProperty.call(this._store, key)) {
            delete this._store[key];
            var idx = this._order.indexOf(key);
            if (idx >= 0) this._order.splice(idx, 1);
            return Promise.resolve(true);
        }
        return Promise.resolve(false);
    };
    Cache.prototype.keys = function() {
        var out = [];
        for (var i = 0; i < this._order.length; i++) out.push(new globalThis.Request(this._order[i]));
        return Promise.resolve(out);
    };

    function CacheStorage() { this._caches = {}; this._names = []; }
    CacheStorage.prototype.open = function(name) {
        name = String(name);
        if (!Object.prototype.hasOwnProperty.call(this._caches, name)) {
            this._caches[name] = new Cache();
            this._names.push(name);
        }
        return Promise.resolve(this._caches[name]);
    };
    CacheStorage.prototype.has = function(name) {
        return Promise.resolve(Object.prototype.hasOwnProperty.call(this._caches, String(name)));
    };
    CacheStorage.prototype['delete'] = function(name) {
        name = String(name);
        if (Object.prototype.hasOwnProperty.call(this._caches, name)) {
            delete this._caches[name];
            var idx = this._names.indexOf(name);
            if (idx >= 0) this._names.splice(idx, 1);
            return Promise.resolve(true);
        }
        return Promise.resolve(false);
    };
    CacheStorage.prototype.keys = function() { return Promise.resolve(this._names.slice()); };
    CacheStorage.prototype.match = function(request) {
        var self = this;
        var names = this._names.slice();
        var i = 0;
        function tryNext() {
            if (i >= names.length) return Promise.resolve(undefined);
            return self._caches[names[i]].match(request).then(function(r) {
                if (r) return r;
                i++;
                return tryNext();
            });
        }
        return tryNext();
    };

    globalThis.Cache = Cache;
    globalThis.CacheStorage = CacheStorage;
    if (globalThis.caches == null) globalThis.caches = new CacheStorage();
})();
"#;
