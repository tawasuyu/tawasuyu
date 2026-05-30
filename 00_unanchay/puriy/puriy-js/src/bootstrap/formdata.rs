pub(crate) const FORMDATA_BOOTSTRAP: &str = r#"
// Fase 7.54 — FormData. Modelo de datos fiel (pares ordenados `[name,
// value, filename]`); el value es string o Blob. API: append/set/get/
// getAll/has/delete/forEach + iteradores keys/values/entries + Symbol.
// iterator.
//
// Divergencia documentada: pasar un FormData como `body` de fetch/XHR NO
// se serializa a `multipart/form-data` todavía — eso requiere boundary +
// Content-Type auto-seteado, que depende del lado nativo (fetch.rs no
// arma multipart). Por ahora `String(formData)` cae al default de Object
// (`[object FormData]`); usar URLSearchParams si se necesita un body
// urlencoded real. El valor de FormData hoy es construir/leer el modelo.
globalThis.FormData = function() {
    this._list = [];
};
globalThis.FormData.prototype.append = function(name, value, filename) {
    name = String(name);
    var v = (value instanceof globalThis.Blob) ? value : String(value);
    this._list.push([name, v, filename != null ? String(filename) : undefined]);
};
globalThis.FormData.prototype.set = function(name, value, filename) {
    name = String(name);
    var v = (value instanceof globalThis.Blob) ? value : String(value);
    var entry = [name, v, filename != null ? String(filename) : undefined];
    var found = false; var out = [];
    for (var i = 0; i < this._list.length; i++) {
        if (this._list[i][0] === name) {
            if (!found) { out.push(entry); found = true; }
        } else {
            out.push(this._list[i]);
        }
    }
    if (!found) out.push(entry);
    this._list = out;
};
globalThis.FormData.prototype.get = function(name) {
    name = String(name);
    for (var i = 0; i < this._list.length; i++) {
        if (this._list[i][0] === name) return this._list[i][1];
    }
    return null;
};
globalThis.FormData.prototype.getAll = function(name) {
    name = String(name);
    var out = [];
    for (var i = 0; i < this._list.length; i++) {
        if (this._list[i][0] === name) out.push(this._list[i][1]);
    }
    return out;
};
globalThis.FormData.prototype.has = function(name) {
    name = String(name);
    for (var i = 0; i < this._list.length; i++) {
        if (this._list[i][0] === name) return true;
    }
    return false;
};
globalThis.FormData.prototype.delete = function(name) {
    name = String(name);
    var out = [];
    for (var i = 0; i < this._list.length; i++) {
        if (this._list[i][0] !== name) out.push(this._list[i]);
    }
    this._list = out;
};
globalThis.FormData.prototype.forEach = function(cb, thisArg) {
    for (var i = 0; i < this._list.length; i++) {
        cb.call(thisArg, this._list[i][1], this._list[i][0], this);
    }
};
globalThis.FormData.prototype.entries = function() {
    return globalThis.__puriy_usp_iter(this._list, function(e) { return [e[0], e[1]]; });
};
globalThis.FormData.prototype.keys = function() {
    return globalThis.__puriy_usp_iter(this._list, function(e) { return e[0]; });
};
globalThis.FormData.prototype.values = function() {
    return globalThis.__puriy_usp_iter(this._list, function(e) { return e[1]; });
};
globalThis.FormData.prototype[Symbol.iterator] = function() {
    return this.entries();
};
"#;
