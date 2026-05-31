pub(crate) const HEADERS_BOOTSTRAP: &str = r#"
// Fase 7.33 — Headers class. Spec: case-insensitive keys, soporta
// múltiples values por key (joined con ', ').
globalThis.Headers = function(init) {
    this._store = {};
    // Fase 7.62 — Set-Cookie se guarda aparte como lista: el spec lo trata
    // distinto del resto (no se puede comma-joinar sin romper Expires=…, y
    // `getSetCookie()` debe devolver cada cookie como elemento separado).
    this._setCookie = [];
    if (init) {
        if (init instanceof globalThis.Headers) {
            for (var k in init._store) this._store[k] = init._store[k];
            this._setCookie = init._setCookie ? init._setCookie.slice() : [];
        } else if (typeof init === 'object') {
            for (var k2 in init) {
                if (Object.prototype.hasOwnProperty.call(init, k2)) {
                    this.set(String(k2), String(init[k2]));
                }
            }
        }
    }
};
globalThis.Headers.prototype.get = function(name) {
    var k = String(name).toLowerCase();
    return Object.prototype.hasOwnProperty.call(this._store, k) ? this._store[k] : null;
};
globalThis.Headers.prototype.set = function(name, value) {
    var k = String(name).toLowerCase();
    this._store[k] = String(value);
    if (k === 'set-cookie') this._setCookie = [String(value)];
};
globalThis.Headers.prototype.append = function(name, value) {
    var k = String(name).toLowerCase();
    if (Object.prototype.hasOwnProperty.call(this._store, k)) {
        this._store[k] = this._store[k] + ', ' + String(value);
    } else {
        this._store[k] = String(value);
    }
    if (k === 'set-cookie') this._setCookie.push(String(value));
};
globalThis.Headers.prototype.has = function(name) {
    return Object.prototype.hasOwnProperty.call(this._store, String(name).toLowerCase());
};
globalThis.Headers.prototype.delete = function(name) {
    var k = String(name).toLowerCase();
    delete this._store[k];
    if (k === 'set-cookie') this._setCookie = [];
};
// Fase 7.62 — getSetCookie() devuelve cada valor Set-Cookie como elemento
// separado (a diferencia de get('set-cookie'), que los comma-joina).
globalThis.Headers.prototype.getSetCookie = function() {
    return this._setCookie.slice();
};
globalThis.Headers.prototype.forEach = function(cb) {
    for (var k in this._store) {
        if (Object.prototype.hasOwnProperty.call(this._store, k)) {
            cb(this._store[k], k, this);
        }
    }
};
globalThis.Headers.prototype.keys = function() {
    return Object.keys(this._store);
};
// Fase 7.59 — protocolo iterable completo (`entries`/`values`/Symbol.iterator).
// El spec ordena la iteración por nombre (los keys ya están en minúsculas);
// reusamos el helper `__puriy_usp_iter` de URLSearchParams. Habilita
// `for (const [k, v] of headers)`, `[...headers]` y `new URLSearchParams(h)`.
globalThis.Headers.prototype.__puriy_pairs = function() {
    var out = [];
    for (var k in this._store) {
        if (Object.prototype.hasOwnProperty.call(this._store, k)) out.push([k, this._store[k]]);
    }
    out.sort(function(a, b) { return a[0] < b[0] ? -1 : (a[0] > b[0] ? 1 : 0); });
    return out;
};
globalThis.Headers.prototype.entries = function() {
    return globalThis.__puriy_usp_iter(this.__puriy_pairs(), function(e) { return [e[0], e[1]]; });
};
globalThis.Headers.prototype.values = function() {
    return globalThis.__puriy_usp_iter(this.__puriy_pairs(), function(e) { return e[1]; });
};
globalThis.Headers.prototype[Symbol.iterator] = function() {
    return this.entries();
};
"#;
