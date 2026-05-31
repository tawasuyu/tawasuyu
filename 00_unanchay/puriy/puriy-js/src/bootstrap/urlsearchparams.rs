pub(crate) const URLSEARCHPARAMS_BOOTSTRAP: &str = r#"
// Fase 7.51 — URLSearchParams. Construible desde string (`'a=1&b=2'`, con
// `?` inicial opcional), objeto plano, array de pares `[[k,v],...]` u otro
// URLSearchParams. Mantiene orden de inserción en `_list` (array de pares
// `[k, v]`). Como `toString()` produce la forma application/x-www-form-
// urlencoded, pasar un URLSearchParams como `body` de fetch/XHR funciona
// sin tocar fetch.rs — el `String(body)` lo serializa solo. Divergencia:
// NO auto-seteamos `Content-Type: application/x-www-form-urlencoded` (el
// browser lo hace; acá el caller lo agrega si lo necesita).
//
// Codec form-urlencoded: espacio → '+', resto percent-encoded UTF-8.
// `encodeURIComponent` deja literales `!~*'()` que el form spec sí encoda,
// así que los forzamos; `* - . _` quedan literales (spec).
globalThis.__puriy_form_encode = function(s) {
    return encodeURIComponent(String(s))
        .replace(/%20/g, '+')
        .replace(/[!'()~]/g, function(c) {
            return '%' + c.charCodeAt(0).toString(16).toUpperCase();
        });
};
globalThis.__puriy_form_decode = function(s) {
    var t = String(s).replace(/\+/g, ' ');
    try { return decodeURIComponent(t); } catch (e) { return t; }
};
globalThis.URLSearchParams = function(init) {
    this._list = [];
    if (init == null || init === '') return;
    if (init instanceof globalThis.URLSearchParams) {
        for (var i = 0; i < init._list.length; i++) {
            this._list.push([init._list[i][0], init._list[i][1]]);
        }
    } else if (typeof init === 'string') {
        var s = init.charAt(0) === '?' ? init.substring(1) : init;
        if (s) {
            var pairs = s.split('&');
            for (var j = 0; j < pairs.length; j++) {
                if (pairs[j] === '') continue;
                var idx = pairs[j].indexOf('=');
                var k, v;
                if (idx < 0) { k = pairs[j]; v = ''; }
                else { k = pairs[j].substring(0, idx); v = pairs[j].substring(idx + 1); }
                this._list.push([globalThis.__puriy_form_decode(k), globalThis.__puriy_form_decode(v)]);
            }
        }
    } else if (Array.isArray(init)) {
        for (var m = 0; m < init.length; m++) {
            this._list.push([String(init[m][0]), String(init[m][1])]);
        }
    } else if (typeof init === 'object' && typeof init[Symbol.iterator] === 'function') {
        // Fase 7.59 — cualquier iterable de pares (Headers, FormData, Map…).
        var it = init[Symbol.iterator]();
        var step = it.next();
        while (!step.done) {
            this._list.push([String(step.value[0]), String(step.value[1])]);
            step = it.next();
        }
    } else if (typeof init === 'object') {
        for (var key in init) {
            if (Object.prototype.hasOwnProperty.call(init, key)) {
                this._list.push([key, String(init[key])]);
            }
        }
    }
};
globalThis.URLSearchParams.prototype.append = function(name, value) {
    this._list.push([String(name), String(value)]);
};
globalThis.URLSearchParams.prototype.delete = function(name, value) {
    name = String(name);
    // Fase 7.66 — overload de dos args: si se pasa value, borra sólo los
    // pares que matcheen nombre Y valor (spec WHATWG reciente).
    var hasValue = arguments.length > 1;
    if (hasValue) value = String(value);
    var out = [];
    for (var i = 0; i < this._list.length; i++) {
        var drop = this._list[i][0] === name && (!hasValue || this._list[i][1] === value);
        if (!drop) out.push(this._list[i]);
    }
    this._list = out;
};
globalThis.URLSearchParams.prototype.get = function(name) {
    name = String(name);
    for (var i = 0; i < this._list.length; i++) {
        if (this._list[i][0] === name) return this._list[i][1];
    }
    return null;
};
globalThis.URLSearchParams.prototype.getAll = function(name) {
    name = String(name);
    var out = [];
    for (var i = 0; i < this._list.length; i++) {
        if (this._list[i][0] === name) out.push(this._list[i][1]);
    }
    return out;
};
globalThis.URLSearchParams.prototype.has = function(name, value) {
    name = String(name);
    // Fase 7.66 — overload de dos args: con value, exige match de nombre Y valor.
    var hasValue = arguments.length > 1;
    if (hasValue) value = String(value);
    for (var i = 0; i < this._list.length; i++) {
        if (this._list[i][0] === name && (!hasValue || this._list[i][1] === value)) return true;
    }
    return false;
};
globalThis.URLSearchParams.prototype.set = function(name, value) {
    name = String(name); value = String(value);
    var found = false; var out = [];
    for (var i = 0; i < this._list.length; i++) {
        if (this._list[i][0] === name) {
            if (!found) { out.push([name, value]); found = true; }
            // ocurrencias extra se descartan (spec: set deja una sola)
        } else {
            out.push(this._list[i]);
        }
    }
    if (!found) out.push([name, value]);
    this._list = out;
};
globalThis.URLSearchParams.prototype.sort = function() {
    // Orden estable por key (code units). Array.sort de QuickJS es estable.
    this._list.sort(function(a, b) {
        return a[0] < b[0] ? -1 : (a[0] > b[0] ? 1 : 0);
    });
};
globalThis.URLSearchParams.prototype.forEach = function(cb, thisArg) {
    for (var i = 0; i < this._list.length; i++) {
        cb.call(thisArg, this._list[i][1], this._list[i][0], this);
    }
};
globalThis.URLSearchParams.prototype.toString = function() {
    var out = [];
    for (var i = 0; i < this._list.length; i++) {
        out.push(globalThis.__puriy_form_encode(this._list[i][0]) + '=' +
                 globalThis.__puriy_form_encode(this._list[i][1]));
    }
    return out.join('&');
};
globalThis.__puriy_usp_iter = function(list, pick) {
    var i = 0;
    return {
        next: function() {
            if (i < list.length) { var e = list[i++]; return { value: pick(e), done: false }; }
            return { value: undefined, done: true };
        },
        [Symbol.iterator]: function() { return this; }
    };
};
globalThis.URLSearchParams.prototype.entries = function() {
    return globalThis.__puriy_usp_iter(this._list, function(e) { return [e[0], e[1]]; });
};
globalThis.URLSearchParams.prototype.keys = function() {
    return globalThis.__puriy_usp_iter(this._list, function(e) { return e[0]; });
};
globalThis.URLSearchParams.prototype.values = function() {
    return globalThis.__puriy_usp_iter(this._list, function(e) { return e[1]; });
};
globalThis.URLSearchParams.prototype[Symbol.iterator] = function() {
    return this.entries();
};
// Fase 7.66 — `size` (cantidad de pares, contando duplicados). Adición
// reciente del spec WHATWG.
Object.defineProperty(globalThis.URLSearchParams.prototype, 'size', {
    get: function() { return this._list.length; }
});
// El valor de retorno de defineProperty es el prototype; cerrar con `void 0`
// evita que el bootstrap coaccione ese objeto a string (su toString corre con
// `this = prototype`, sin `_list`, y tiraría).
void 0;
"#;
