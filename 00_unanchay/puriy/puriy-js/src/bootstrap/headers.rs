pub(crate) const HEADERS_BOOTSTRAP: &str = r#"
// Fase 7.33 — Headers class. Spec: case-insensitive keys, soporta
// múltiples values por key (joined con ', ').
globalThis.Headers = function(init) {
    this._store = {};
    if (init) {
        if (init instanceof globalThis.Headers) {
            for (var k in init._store) this._store[k] = init._store[k];
        } else if (typeof init === 'object') {
            for (var k2 in init) {
                if (Object.prototype.hasOwnProperty.call(init, k2)) {
                    this._store[String(k2).toLowerCase()] = String(init[k2]);
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
    this._store[String(name).toLowerCase()] = String(value);
};
globalThis.Headers.prototype.append = function(name, value) {
    var k = String(name).toLowerCase();
    if (Object.prototype.hasOwnProperty.call(this._store, k)) {
        this._store[k] = this._store[k] + ', ' + String(value);
    } else {
        this._store[k] = String(value);
    }
};
globalThis.Headers.prototype.has = function(name) {
    return Object.prototype.hasOwnProperty.call(this._store, String(name).toLowerCase());
};
globalThis.Headers.prototype.delete = function(name) {
    delete this._store[String(name).toLowerCase()];
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
"#;
